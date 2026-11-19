# Design

## Context

See proposal.md for the five items and why. What the code does today, as far as it shapes the approach:

- **Profile menu** (`src/workspace.rs`): `profile_menu_entries` turns the picker's rows into menu items keyed by `ProfileKey` text (`"0:Shell"`), or by index when a key is missing. Its doc says there are no separators, so an item's index is its profile's. The `MenuEvent::Selected` handler parses the key as a `ProfileKey` and falls back to the item's index. A non-profile item would therefore open the profile at its index.
- **Theme list** (`settings_dialog/themes.rs`): `ThemeEntry { name, custom: bool, theme }`. Custom rows get a muted "custom" tag; built-ins get none. `enabled(Btn::Delete)` is already false on a built-in, and `request_delete` refuses with a reason. `start_edit` on a built-in calls `start_new`, so Edit and New do the same thing there.
- **Icons** (`ui/src/icon.rs`): `brindle-ui` draws lucide's shapes itself as unit-square `Stroke`s (`Polyline`, `Circle`, `Rect`) on lucide's 24 grid. Its 14 icons are listed in `Icon::ALL`, which the table-driven tests and the gallery iterate. It has no lock.
- **Tooltips**: `brindle_ui::TooltipFor::tooltip_for(label, shortcut)` attaches a `Tooltip` to any stateful element (one with an `id`), as the zone headers and the strip do.
- **Close `X`** (`settings_dialog.rs`): `Dialog::on_close` calls `request_cancel` → `Model::cancel()`. That closes from a list or the Fonts form but only leaves an editor or the delete question. ctrl-shift-, and opening a palette already close the whole dialog through `Workspace::close_overlay`, dropping an editor's edits without asking. The Cancel buttons emit `DismissEvent`, which reaches the same path.
- **Selection and paste** (`terminal_view.rs`, `terminal/mod.rs`, `terminal/mouse.rs`): `mouse_down` reports to the program when `MOUSE_MODE` is on and shift isn't held. Otherwise left selects, middle pastes the primary selection, and right does nothing (`_ => {}`). `Terminal::finish_selection` copies to the clipboard when its `copy_on_select` field is set, and that field is snapshotted from the config when the terminal is made, so a reload never reaches open tabs. `--click` only presses the left button (`PointerStep { click: bool }`).
- **Settings sections**: Profiles, Theme and Fonts. Fonts is a form whose model, `FontsState`, reads its values from the file's text past errors elsewhere and returns a `ConfigEdit::SetFonts` on Apply.
- **Fault switches** (`process R20`): `BRINDLE_TEST_SINK_PANIC` (`terminal/server.rs`) and `BRINDLE_TEST_LAUNCH_LISTING` (`main.rs`) in Brindle, and `BRINDLE_TEST_HOLDER_PANIC` and `BRINDLE_TEST_FEED_PANIC` (`server/fault.rs`) in the helper, each read only under `cfg(debug_assertions)`. These e2e cases set them: `local-server-sink-panic`, `local-server-session-error`, `local-server-reattach-slow-listing` and `local-server-reattach-failed-listing`. Under `cargo test --release --test e2e` they fail, after a full GUI run, with hints that guess at the cause. The real-process tests that set the helper's switches (`holder.rs`'s `faulty_holder` tests, `core.rs`'s `a_panicking_emulator_…`) would fail the same way under `cargo test --release`. The e2e harness picks its binaries at run time (`BRINDLE_E2E_BIN`, `BRINDLE_REMOTE_AGENT`/`BRINDLE_SERVER_HELPER`), so its own `cfg!(debug_assertions)` says nothing reliable about them.

## Goals / Non-Goals

**Goals:**
- Every decision lands in a pure function or GPUI-free model with unit tests (`fcis R1`, `R8`), and every visible behaviour gets a named e2e case (`testing R1`, `R13`).
- A test that needs a fault switch finds out, before it runs, whether its binary has one, and says so when it doesn't.

**Non-Goals:**
- A context menu in the terminal area. `right_click` is a string key so that a later `"menu"` can join it, but this change adds no such variant (`types R12`).
- `osc52_paste` or other behaviour keys in the settings dialog.
- Changing `--version` output in either binary.
- Folding `BRINDLE_REMOTE_E2E_REQUIRED` into a general list (see Open Questions).

## Decisions

### Layer map

| Item | Pure (tested in its file) | Shell (gathers, calls, applies) | Crate |
| --- | --- | --- | --- |
| 1 Profile menu | `profile_menu_entries` (now with the separator and the Settings item); `profile_menu_choice` → `ProfileMenuChoice` | the menu's `MenuEvent` subscription in `Workspace::new` | `brindle` |
| 2 Lock icon | `Icon::Lock`'s strokes, pinned by `icon.rs`'s table-driven tests | `IconElement` paints it, unchanged | `brindle-ui` |
| 2 Built-ins | `ThemesState::enabled`, `start_edit`, `request_delete`; `ThemeEntry::tag` | `SettingsDialog::render_list` (the lock and its tooltip) | `brindle` |
| 3 Close `X` | `Model::close` → `Step::Close` from every view | `Dialog::on_close` → `SettingsDialog::step` | `brindle` |
| 4 Right-click | `config::RightClick` (parsed at the config boundary); `mouse::reports`, `mouse::right_press` (setting × selection) → `RightPress`; `PointerStep::parse` with `PointerAction` | `TerminalView::mouse_down` and `mouse_up` read `Settings` and apply; `main.rs` parses `--right-click` | `brindle` |
| 4 Settings control | `settings_dialog/clipboard.rs` `ClipboardState`; `config_edit` reader and `ConfigEdit::SetClipboard` | the `SettingsDialog` view's two `Switch`es | `brindle` (uses `brindle_ui::Switch`, unchanged) |
| 5 Switch lists | `fault_switches()` in `main.rs`; `fault::honoured()` in the helper | `--list-fault-switches` in `main.rs`'s argument loop and the helper's `server/cli.rs` | `brindle`, `brindle-remote-agent` |
| 5 Harness | `tests/e2e/switches.rs`: `TestSwitch`, `parse_listing`, `readiness`, `undeclared` (its `--self-test`, `testing R3`); `remote-agent/tests/support/wait.rs` `Capability::FaultSwitches`, `parse_listing` (`#[test]`s) | the e2e runner's probe and per-case gate; `Run::env`'s guard; the support crate's probe | test code |

No crate reference changes (`ports R1`). `brindle-ui` gains one icon, a drawing that names no app concept, so its boundary holds (`ports R2`): the theme row, the word "built-in" and the tooltip's text stay in the app. No platform `cfg` is added outside a seam (`ports R4`): the clipboard goes through GPUI on every platform, and the helper's new command sits in `server/cli.rs` beside `list` and `status`, so `remote-agent/src/main.rs` gains no new `cfg` site. `cfg(debug_assertions)` is no platform `cfg`, and the lists use `cfg!(debug_assertions)` in one expression rather than a pair of items. No wire message, state file or terminal-server behaviour changes, so `process R1`–`R6` and `R12` aren't touched.

### 1. The profile menu's Settings item

The menu's entries become `[profile items…, Separator, Settings]`. Settings is keyed by a reserved `ItemKey`, `"settings"`, which can't be a `ProfileKey` (no `:`) or an index (not digits). Unit tests pin both. The handler stops parsing keys itself and asks a pure decoder:

```rust
enum ProfileMenuChoice { Profile { key: Option<ProfileKey>, listed: usize }, Settings }
fn profile_menu_choice(entries: &[MenuEntry], key: &ItemKey) -> Option<ProfileMenuChoice>
```

`Settings` can't carry a profile index, and a profile choice can't be the Settings item (`types R1`). `Profile` behaves as today: the profiles come first, so `listed` is still the profile's index. `Settings` calls `open_settings`. The subscription already runs after the menu has closed and given focus back to the tab, so that is the focus the dialog records and returns to. The item's shortcut comes from the keymap, as the command palette's does (`shortcut(&OpenSettings, …)`), and is passed to `profile_menu_entries` as a value. The item is labelled "Settings", as the user asked.

*Alternatives:* a `MenuEntry` carrying a boxed action would push app concepts into `brindle-ui` (`ports R2`). Matching on the label would break the menu's typeahead and a translated label.

### 2. Built-in themes are marked and read-only

- **The marker is a lock icon.** `brindle-ui` gains `Icon::Lock`, lucide's `lock` (`rect width=18 height=11 x=3 y=11 rx=2`, `M7 11V7a5 5 0 0 1 10 0v4`), in unit strokes:

  ```rust
  // lucide `lock`: the body, then the shackle with its arc as eight chords
  const LOCK: &[Stroke] = &[
      rect(3., 11., 18., 11., 2.),
      Stroke::Polyline(&[
          p(7., 11.), p(7., 7.),
          p(7.38, 5.09), p(8.46, 3.46), p(10.09, 2.38), p(12., 2.),
          p(13.91, 2.38), p(15.54, 3.46), p(16.62, 5.09),
          p(17., 7.), p(17., 11.),
      ]),
  ];
  ```

  The arc points are the radius-5 semicircle about (12, 7) every 22.5°, rounded to hundredths. `Stroke` has no arc, and a polyline keeps the painter unchanged. Its joins turn by 22.5° at most, so the miter joins stay close to the line. `Icon::ALL` grows to 15, so `every_icon_builds_at_component_sizes`, `every_icon_stays_inside_its_unit_box` (the shackle's top at y = 2 is 1 unit clear of the edge at lucide's stroke) and the gallery's icon row take it with no new code. `shapes_are_lucides` gains a lock check: the body's rectangle, the shackle's end points and top, and every arc point within 0.01 of radius 5 from (12, 7).
- **The row.** Each built-in row shows `Icon::Lock` in the muted foreground at 0.875rem, where custom rows show their "custom" tag. It sits in a `div` with an `id` (`("theme-lock", ix)`), so `tooltip_for("Built in: New makes an editable copy", None)` attaches, with no shortcut. The tooltip is pointer-only. The keyboard path gets the same words from the refusal `e` and `d` show.
- **The text marker stays in the model.** `ThemeEntry::tag()` returns `"built-in"` or `"custom"`, and the dump's list line prints the selected entry's as ` origin=built-in|custom`. That is what the e2e case reads, since the dump carries no icons. GPUI 0.2.2 has no accessibility tree to give it to, so the tag is also the name an accessible row would take once one exists.
- `enabled(Btn::Edit)` is true only for a readable custom theme. `start_edit` on a built-in returns `Reply::Refused("\"tokyo-night\" is built in: New makes an editable copy")` instead of calling `start_new`. `request_delete`'s refusal becomes `"… is built in and can't be deleted"`. Both show in the section's error line, as refusals do today (`railway R7`: a "no" is an answer, not an error).
- New is unchanged: it copies the selected theme, built-in or not.

*Decision on blocking edits:* yes. Edit on a built-in already only made a copy, which is New's job, so disabling it loses nothing and stops the editor opening under a different name than the one the user picked. The refusal names the way forward.

*Alternatives:* a "built-in" text tag beside the "custom" one (the user chose the icon); an SVG asset for the lock (`brindle-ui` draws every icon from strokes and ships no assets); relabelling Edit as "Copy" on built-ins (two buttons doing the same thing); group headings for "Built-in" and "Custom" (more layout for the same information, and the list's keyboard model indexes entries, not headings). `ThemeEntry`/`ThemeKey` keep `custom: bool`: one flag breaks no `types` rule, and changing it would touch every theme test for no behaviour.

### 3. The close `X` closes the dialog

`Model::close()` returns `Step::Close` from every view, a dirty editor and the delete question included. `Dialog::on_close` and the focused `X`'s enter and space take that step. Escape keeps `Model::cancel()`'s step-back behaviour. `Step::Close` emits `DismissEvent`, and the workspace's existing close path ends the theme, profile and font previews and returns focus to the tab. Nothing new is written there. A unit test runs `close()` over every `View` with edits pending, so the rule is reachable without a window (`testing R1`).

*Unsaved edits:* they are dropped without asking. That is already what ctrl-shift-, and opening a palette do to an open editor, so the `X` now matches them. The old rule that the `X` acts as escape so it "never loses an editor's edits silently" is replaced in the spec. A confirmation would be a fourth `ListMode` with its own buttons, stops and dump line, which nothing else in the dialog asks for yet (see Open Questions).

### 4. Right-click paste, and both clipboard settings read live

**Config** (`config.rs`): `right_click: RightClick`, where `enum RightClick { Nothing, Paste }` deserialises from `"none"` and `"paste"`, with `Nothing` the default. It is parsed once at the config boundary (`types R6`). An unknown value is a config error like any other: defaults apply and the banner says why (`process R15`, `railway R10`).
- *Which versions meet:* the key is additive in meaning, since its absence is today's behaviour. A Brindle that predates it rejects it as an unknown key (banner, defaults), as it would any newer key, so the settings dialog writes it only when the user changes it.
- *Why a string:* a later right-click menu would exclude paste. A bool `paste_on_right_click` would then need a second, exclusive bool in the file. Both of the enum's variants mean something today (`types R12`).

**Decision** (`terminal/mouse.rs`, pure, beside the encoders):

```rust
pub fn reports(mode: TermMode, shift: bool) -> bool          // moved out of TerminalView::reporting
pub enum RightPress { Report, Paste, Nothing }
pub fn right_press(reports: bool, has_selection: bool, setting: RightClick) -> RightPress
```

`Report` wins whenever the program reports and shift isn't held, so `RightPress` can't both report and paste. Otherwise the press is `Paste` only when the setting is `Paste` and nothing is selected, and `Nothing` in every other case. Its tests cover every combination of reports, selection and setting.

*Decision: a right press over a selection does nothing.* It neither pastes nor copies, and the selection stays.
- It prevents a stray paste over a selection. A user who has just selected text is about to copy it or read it, and a paste there types into the program while the screen still shows the selection.
- A copy on right-click would add nothing with `copy_on_select` on, because the selection is already in the clipboard. With it off, a copy would give the right button two meanings that depend on what's on screen.
- A left click clears the selection, and the next right press pastes.
- *Shift over a reporting program:* shift makes `reports` false, so a shift-right-click meets the same selection rule as a right press in a program that doesn't report. A shift-drag selects there and the following shift-right-click does nothing, which is the pair the rule is for. No separate case exists to drift.
- *Alternatives:* PuTTY's "always paste" (it pastes over a selection, the stray paste this rule exists to stop). Windows Terminal's "copy when selected, else paste" (the right button's meaning would change with the screen, and with `copy_on_select` the copy is redundant).

**Shell** (`terminal_view.rs`): `mouse_down` reads `Settings::get(cx).config.right_click` at the press, and `has_selection` from `terminal.read(cx).selection_text().is_some()`. That is the test the copy action and `finish_selection` use, so "selected" means there is text a copy would take: a click's empty selection doesn't count. It then dispatches on `RightPress`. `Paste` calls the same path as the paste action (`cx.read_from_clipboard()` → `Terminal::paste`, which brackets and strips ESC). An empty clipboard pastes nothing. `Nothing` returns at once: it starts no selection, leaves the existing one, and touches neither clipboard. Neither `Paste` nor `Nothing` sets a `reported_button`, so the release reports nothing. `mouse_up` passes `config.copy_on_select`, read at the release, to `Terminal::finish_selection(to_clipboard: bool, cx)`, and `Terminal` loses its snapshot field. A reload therefore reaches open tabs with no code in `apply_config`. The view already reads `Settings` in handlers (`cursor.blink`), and reading a global in an event handler is the shell's job (`fcis R3` binds only the pure code).

**Debug hook:** `PointerStep { modifiers, col, row, action: PointerAction }`, with `enum PointerAction { Hover, Click(PointerButton) }` and `enum PointerButton { Left, Right }`, replaces `click: bool` (`types R1`: a hover has no button). `--right-click POS` parses through `PointerStep::parse` and is refused before the run like `--click` (`command-line`). `debug_pointer` sends the step's button to `mouse_down` and `mouse_up`. There is no middle-button variant, since no case needs one (`types R12`).

### 5. Clipboard section

`settings_dialog/clipboard.rs` holds `ClipboardState`, modelled on `FontsState`:
- `saved` and `current` are both `ClipboardValues { copy_on_select: bool, right_click: RightClick }`, plus `read_only: Option<String>` for a file that doesn't parse, in which case the loaded config's values are shown read-only.
- The values are read from the file's text by a `config_edit` reader that ignores the rest of the file, as `custom_themes` does. A key of the wrong type reads as its default, and the section's error line names it.
- `toggle(ClipboardField)`, where `enum ClipboardField { CopyOnSelect, RightClick }`.
- `apply()` returns `Ok(Step::Write(ConfigEdit::SetClipboard(changes)))`, with only the changed fields as `enum ClipboardChange { CopyOnSelect(bool), RightClick(RightClick) }`. With no change it returns `Step::Close`. When `read_only` is set it returns `Err`.
- `reset()` runs on leaving the section.
- `Section::Clipboard` and `View::Clipboard` (`ModeKind::Form`) join the closed enums, so every `match` on them must decide (`types R2`). The footer is `[Cancel, Apply]`. Enter applies, and space toggles the focused `Switch`.
- The dump line: `--- settings section=Clipboard mode=form copy_on_select=B right_click=none|paste field=… can_apply=B error=…`.
- `config_edit` writes the top-level keys in place, keeping comments and order (the `configuration` spec's "Writes from Brindle preserve the file"). A failed write keeps the dialog open with the reason, as the other sections do.

*Two switches, not one:* `copy_on_select` already stands alone, and select-to-copy with middle-click paste is a common Linux habit. One switch writing two keys would need a third state when the file sets them differently.

*Name:* "Clipboard", since both settings decide what reaches or leaves the clipboard.

### 6. Fault switches: listed by the binary, declared by the test

**The binaries say what they honour.** `brindle --list-fault-switches` prints `fault_switches()` and `brindle-remote --list-fault-switches` prints `fault::honoured()`, one name per line, and both exit 0. The flag is handled in `main.rs`'s argument loop beside `--list-actions`, so it runs before GPUI, the config and the server. Each list is one `const`-like function, `if cfg!(debug_assertions) { &[A, B] } else { &[] }`, over the same name constants the switches' readers use (`terminal::server::SINK_PANIC`, `main.rs`'s `LAUNCH_LISTING`, `fault::HOLDER_PANIC`, `fault::FEED_PANIC`), so a name can't differ between reader and list. Unit tests pin each list against `cfg!(debug_assertions)` in the same compilation. The relay timings and `BRINDLE_TEST_HOLDER_CORE_ARGS` aren't fault switches: release builds honour them, and they stay unlisted.

*Alternatives considered:*
- The harness's own `cfg!(debug_assertions)`: wrong whenever `BRINDLE_E2E_BIN` or a helper override names another build, and it is an inference, not the fact (`testing R11`).
- A profile word in `--version`: `--version` output is a contract (CI compares `brindle-remote --version` byte for byte), and "debug" is still an inference about switches.
- A `--- build` dump line: it needs a window and arrives after the case has run, so a missing switch would cost a full GUI run and then be reported after the fact.

**The e2e harness decides before a window opens.** In `tests/e2e/switches.rs`:
- `enum TestSwitch { SinkPanic, LaunchListing, HolderPanic }` has `name()` and `host() -> SwitchHost { Brindle, Helper }`. It holds only the switches cases use; the helper's `FeedPanic` is used only by the real-process tests (`types R12`).
- `Case` gains `needs: &'static [TestSwitch]`, and the four cases declare theirs in `cases/mod.rs` and `cases/win.rs`.
- The runner probes each host at most once, lazily, and only when a selected case needs it. It runs `<bin> --list-fault-switches` through `common::server::isolate` with a scratch directory, waits through `Scale` with a 10 s limit, and kills the process past that.
- A probe is `Result<BTreeSet<String>, String>`. An older binary that rejects the flag, a crash or a timeout is an `Err` naming the binary and its stderr or exit status (`railway R1`, `R8`).
- `readiness(needs, &brindle, &helper, required) -> Readiness { Run, Skip(String), Fail(String) }` is pure. A missing switch gives `Skip` (or `Fail` when `BRINDLE_E2E_SWITCHES_REQUIRED=1`), with a reason naming the switch, the binary's path, and how to get one that has it: `needs BRINDLE_TEST_SINK_PANIC, which /…/target/release/brindle doesn't list (a release build has none: run without --release, or point BRINDLE_E2E_BIN at a debug build)`. It runs after the display check and before the case's directories fill, so no window opens. The run's summary counts it as skipped.
- **Guard:** `Run::env` with a `BRINDLE_TEST_*` key that its case didn't declare makes `run()` return `Err` before launching (`undeclared(key, needs) -> Option<String>`, pure). A future case can't repeat the mistake silently.
- The cases' guessing hints ("is BRINDLE_E2E_BIN a release build?", "(the fault switch is in debug builds only)") are removed, since the gate states the cause.

**The real-process tests use the skip they already have.** `Capability::FaultSwitches` (`"fault-switches"`) joins `Capability::ALL`. `support` probes `BIN --list-fault-switches` once (a `OnceLock`), and the holder fault tests and `a_panicking_emulator_…` call `skip(Capability::FaultSwitches, reason)` and return when their switch isn't listed. `BRINDLE_TEST_REQUIRE=fault-switches` turns that into a failure.

**CI requires it, so a misconfigured job fails rather than skipping.** `ci.yml`'s `test` job and `build-windows.yml`'s `cargo test` step set `BRINDLE_TEST_REQUIRE: fault-switches`, and the `e2e` and `windows-e2e` jobs set `BRINDLE_E2E_SWITCHES_REQUIRED: "1"`.

**Rules:** this keeps `testing R2` (the probe is a CLI query, not input), `R4` (isolated), `R7` (a bounded wait through the helper, no sleep) and `R9` (no retry; a skip is for a missing capability, not a flake), and `process R20`. The pattern is written down as a new `testing R18`, and `process R20` gains the listing.

### Rules relied on

`fcis R1`, `R3`, `R5`, `R8`; `types R1`, `R2`, `R6`, `R12`; `railway R1`, `R6`–`R8`, `R10`; `ports R1`, `R2`, `R4`; `process R13` (nothing new on the UI thread: the clipboard read is GPUI's own synchronous call, as the paste action makes today), `R15`, `R20`; `testing R1`–`R4`, `R7`, `R9`, `R11`, `R13`, `R16`. No exception is needed.

## Risks / Trade-offs

- [The `X` drops unsaved editor edits.] → It matches ctrl-shift-, and the palettes. Escape still steps back. The README says so. A confirmation stays open as a follow-up.
- [Right-click paste fires on a stray right-click and runs a pasted command ending in a newline, when the program doesn't bracket pastes.] → It is off by default, and it never fires while text is selected. It takes the paste key's path, so bash and zsh, with bracketed paste on, don't run it until enter.
- [Users from PuTTY expect a right-click to paste over a selection, and here it does nothing.] → A left click clears the selection. The README row, the default config's comment and the Clipboard section's hint say "when nothing is selected".
- [The lock's arc is eight chords with miter joins, not lucide's round path.] → At 14px each chord is about a pixel long, so the arc reads as round. `shapes_are_lucides` pins the points to the radius.
- [A program that reports the mouse gets right presses, so right-click paste "doesn't work" in htop or vim.] → It follows the shift-drag precedent. The spec and README say to hold shift.
- [A real right press on Windows could raise the system menu.] → The terminal area is client area with no caption hit, but only a real pointer shows it: a manual check on Linux and Windows.
- [The probe adds a process start per host per run.] → It is lazy, once per host, a sub-second exit before GPUI, and only when a selected case needs it.
- [A new fault switch is added without being listed.] → A case that declares it is skipped in debug runs too, and CI's require variables fail it, so the omission shows on the first CI run. `process R20`'s new wording makes listing part of the rule.
- [A section added to the settings dialog shifts the e2e cases that count sections or wrap.] → `settings-sections`, `settings-section-list` and `settings-section-shortcuts` are updated in the same group.

## Migration Plan

Nothing to migrate: `right_click` defaults to today's behaviour, and `copy_on_select` keeps its meaning (it now also follows reloads). Each group is its own squash-merged PR, and reverting one reverts its behaviour. Config files that gained `right_click` load with a banner in an older Brindle, as any newer key does.

## Open Questions

These are defaults taken where the request left room. Each can change later without reshaping the specs' structure or the tasks.

- **Confirm before the `X` drops unsaved edits?** Default: no, matching ctrl-shift-, and the palettes. A "Discard unsaved edits?" step would be a follow-up across all three closers.
- **One switch for "select to copy and right-click to paste", or two?** Default: two (`copy_on_select`, `right_click`). The user asked for one option, so this is the default most worth confirming.
- **Section name and item label:** "Clipboard" for the section, and "Settings" (no ellipsis) for the menu item.
- **One require list for the e2e suite:** fold `BRINDLE_REMOTE_E2E_REQUIRED` and `BRINDLE_E2E_SWITCHES_REQUIRED` into `BRINDLE_E2E_REQUIRE=remote,fault-switches`, like `BRINDLE_TEST_REQUIRE`? Deferred, to keep CI's existing variable working.
