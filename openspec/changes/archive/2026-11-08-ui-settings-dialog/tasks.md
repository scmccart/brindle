# Tasks

## 1. brindle-ui: outside presses

- [x] 1.1 Add `Overlay::ignore_outside_press()` (modal slots) and skip the outside-press dismissal for such a layer, with a unit test on the pure decision. Verify: `cargo test -p brindle-ui`.
- [x] 1.2 Add the `ui-components` scenarios to the gallery `-- --check` run (press on the backdrop, Escape, menu inside). Verify: `cargo run -p brindle-ui --example gallery -- --check` prints `ok` for them (opens a window: ask first).

## 2. Workspace hosts the dialog in a slot

- [x] 2.1 Replace `Overlay::Settings` with `settings: ui::Overlay<SettingsDialog>` (modal, ignoring outside presses); open, close, toggle-on-shortcut and the one-overlay rule with palettes go through it, and the slot renders last in the window tree. Verify: `cargo test --workspace` and the existing `settings-*` e2e cases pass under `scripts/with-xvfb`.
- [x] 2.2 Make `SettingsDialog` a `ManagedView` (`DismissEvent`, replacing `SettingsEvent::Close`) and keep preview teardown on close. Verify: `settings-cancel` e2e still restores the theme and returns focus to the tab.

## 3. Dialog chrome and controls

- [x] 3.1 Render through `Dialog` (title "Settings", close X, 760x460) and read colours from the UI tokens; delete `field_box()`/chrome helpers no longer used. Verify: `cargo build` has no warnings and the gallery-size panel looks right in a `--dump-screen-after` capture (ask first).
- [x] 3.2 Replace `button()` with `Button` variants and `field_box()` + `TextInput` with `Input` (invalid state for bad colours, names, numbers). Verify: `settings-*` and `input-settings-tab` e2e cases pass.
- [x] 3.3 Replace the profile editor's `checkbox()` with `Checkbox` and every `render_*_choice` row (and `open_choice`) with `Select`, applying through `SelectEvent::Changed`. Verify: unit tests for the model's choice stepping pass; `settings-profile-save` e2e passes.

## 4. Focus and keys

- [x] 4.1 Implement `FocusStops` per section and mode and drop `next_field`/`prev_field`, `Editor::focused` and `sync_focus_from_window`; derive the `field=` and `--- input` dump values from focus. Verify: `input-settings-tab` passes with unchanged dump formats.
- [x] 4.2 Handle `Dismiss` as the dialog's cancel (menu first, then discard/cancel/close) and remove the escape and tab bindings from the `SettingsDialog` context. Verify: new unit tests on the model's escape outcome per mode.

## 5. Strict shadcn behaviour for selects

- [x] 5.1 Remove left/right on choice fields: the `SettingsChoice` context and its bindings, the `SettingsChoiceNext/Prev` actions and their `--action` names, and the dialog's stepping code (`session_end_next/prev`, `choice_next/prev`), keeping `SelectEvent::Changed` as the only way a choice changes. Update the e2e cases that used `settings_choice_*` to pick through the menu (open with `list_down`/`picker` actions as available, then confirm). Verify: `cargo build` has no warnings and the settings e2e cases pass.
- [x] 5.2 Pin the behaviour: unit tests of the select trigger's key table (enter, space, up, down open; left, right, home, end do nothing), a gallery `-- --check` case for the same, and a "Deviations from shadcn" list in `ui/README.md` covering each existing difference with its reason (audit every component against its shadcn counterpart and list what differs). Verify: `cargo test -p brindle-ui`, the gallery check under `scripts/with-xvfb`, and the list reviewed.

- [x] 5.3 Switch toggles on enter as well as space (Radix's switch is a plain button); the checkbox keeps blocking enter. Update `TOGGLE_KEYS`/the activation tables, their unit tests and the gallery cases, and remove the entry from the README's deviation list. Verify: `cargo test -p brindle-ui` and the gallery check under `scripts/with-xvfb`.
- [x] 5.4 Select pointer behaviour as in Radix: the trigger opens on press, the menu picks on release (press-drag-release is one gesture; a release over the trigger without moving leaves the menu open). Put the decision in pure, unit-tested functions, add gallery cases (they skip on Linux where pointer presses aren't posted), and update the README. Verify: `cargo test -p brindle-ui`; the gallery check under `scripts/with-xvfb` has no failures.
- [x] 5.5 A press outside an open select menu only closes the menu and does not reach the control beneath (the menu layer blocks pointer events outside itself while open, as Radix does). This must keep the spec's nested-overlay rules working: a press inside a higher overlay is not outside the one below, and an outside press with a dialog and menu open closes only the menu. Add a gallery case and remove the README entry. Verify: `cargo test -p brindle-ui` and the gallery check.

## 6. e2e and spec

- [x] 6.1 Add e2e cases `settings-tab-wrap` (with down doing nothing in a text field) and `settings-escape-menu` (Escape with a select menu open closes only the menu; a second Escape leaves the editor), plus focus-returns-to-tab after close (`--- focus terminal` in `settings-cancel`, including a dialog opened over the command palette, `settings-apply` and `settings-profile-preview`). Enter on a focused button is covered by the unit tests `enter_on_a_button_is_the_buttons` and `enter_on_a_dialog_button_reaches_the_button`, by the e2e case `settings-enter-on-button` once 7.1 added `--key` (gpui's own key dispatch, in-process), and by a manual check on a real keyboard (open the theme editor on a custom theme, change a color, tab to Discard and press enter: the editor discards, the list shows and `config.toml` is unchanged; then in a profile editor, change a field, tab to Discard and press enter: nothing is saved; and in the theme list, move to another theme, tab to Cancel and press enter: the dialog closes with the old theme and nothing applied). Verify: `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- settings` passes; the manual check is handed to the user.
- [x] 6.2 Update README keybinding notes if tab/escape behaviour is described. Verify: `grep -n settings README.md` reviewed; manual IME check handed to the user.
- [x] 6.3 Run the full gate. Verify: `cargo test --workspace`, `cargo build --release --locked` and the full e2e suite under `scripts/with-xvfb` are green.

## 7. Focus indicators and list stops

- [x] 7.1 Call `brindle_ui::init` at startup, so a key brings focus rings back after a pointer press; count a section click as pointer input. Make the theme and profile lists one Tab stop each (`list_handle`, `SettingsList` context with up/down, ctrl-p/ctrl-n, home, end), focused when the list opens, with a focus ring; make `FocusRing` public. Add the debug steps `--key` and `--modality`, `stop=` and `modality=` on `--- focus`, and the e2e cases `settings-focus-rings` (General select on open, after a pointer press and a key, the profile list, after Profiles and back), `settings-editor-select-ring` (Tab reaches the Remote select and rings it), `settings-list-stop` (list keys, Tab to the first button, the list's ring) and `settings-enter-on-button` (enter on Discard and on Cancel, by `--key`). Verify: `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- settings input-settings` passes, `settings-focus-rings` fails with the `init` call removed, and the unit tests `settings_list_keys_are_the_lists` and `home_and_end_select_the_lists_edges` pass.
- [x] 7.2 Make the section list the dialog's first Tab stop, a vertical tab list (`sections_handle`, `SettingsSections` context): up/down (wrapping), home/end change the section at once with focus staying on the list, the shown section rings, enter does nothing there, an unsaved editor holds the section and leaving Theme drops its preview. Keep ctrl-tab, ctrl-shift-tab and ctrl-pageup/down. Add the e2e cases `settings-section-list` (shift-tab from General reaches the list and it rings; down, down shows Profiles then Theme; tab goes into Theme's list; end, home, up wrap; an unsaved editor holds; Theme's preview is dropped) and `settings-section-shortcuts` (each shortcut from a select, a list, a text field and a button), and update `settings-tab-wrap` and `settings-enter-on-button` for the new first stop. Verify: `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- settings input-settings` passes and the unit tests `section_keys_step_wrap_and_reach_the_ends` and `settings_list_keys_are_the_lists` pass.
