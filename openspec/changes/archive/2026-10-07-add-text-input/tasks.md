# Tasks

## 1. Editing core

- [x] 1.1 Create `src/text_input.rs` with `EditState` (text, selection range, reversed flag, marked range): insert/replace at the selection with line breaks turned into spaces, char and word movement, home/end, extend-selection variants, backspace/delete/delete-word-back, `select_word_at`, and UTF-8 ↔ UTF-16 offset and range conversion. Verify with `cargo test text_input`, including the spec scenarios (insert in the middle, `café` backspace, ctrl-left twice on `tmux -L work`, ctrl-w on `~/src/brindle`, shift-home then type, multi-line paste) and conversions with a multi-byte and an astral-plane character.
- [x] 1.2 Add marked-text bookkeeping to `EditState` (set, replace on commit, unmark), and verify with unit tests that committing composition replaces the selection and that the offsets stay on char boundaries.

## 2. TextInput entity and element

- [x] 2.1 Add the `TextInput` entity (state, `FocusHandle`, `Changed` event, `Focusable`) and the editing actions in a `TextInput` key context per design Decision 2, registered in `ACTIONS` unlabelled as `text_input_*`. Verify that `cargo build` passes, `brindle --list-actions` lists the new names, and the `actions.rs` label-uniqueness tests pass.
- [x] 2.2 Add `TextInputElement`: shape in prepaint, paint the selection, text, underlined marked text and the caret (focused only), keep the caret scrolled into view, and register `window.handle_input`. Implement `EntityInputHandler` on `TextInput` (text, selection and marked ranges, replace and replace-and-mark, `bounds_for_range`, `character_index_for_point`). Verify with `cargo build` and a unit test of the caret-scroll arithmetic.
- [x] 2.3 Add mouse handling: click focuses and places the cursor via the shaped line, drag selects, and double-click selects a word. Add clipboard copy, cut and paste through `cx.write_to_clipboard`/`read_from_clipboard`. Verify with unit tests of the press/drag selection logic on `EditState`. Interactive checks go in task 4.3.

## 3. Adopting the input

- [x] 3.1 Replace the palette's `TextField` with `Entity<TextInput>` for the filter and the prompt: re-filter on `Changed`, delegate focus to the input, pre-select a prompt's pre-filled value, and remove the palette's `key_down` text path and its `Paste` bindings. Verify that the existing `picker` unit tests pass after adjusting them to `EditState`, and that `cargo build` passes.
- [x] 3.2 Replace the settings dialog's `TextField`s with one `TextInput` per field. Push text into `Model` on `Changed` and write model-initiated changes (clearing selection foreground) back. Give fields tab-stop focus handles, make `SettingsNextField`/`SettingsPrevField` call `focus_next`/`focus_prev`, record the focused field from a focus observer, and remove `Model::key`'s text path, `TextField::is_edit_key` and the dialog's `Paste` bindings. Verify that the `settings_dialog` unit tests pass after porting them to string setters.
- [x] 3.3 Delete `TextField` and update the module docs (`picker.rs`, `settings_dialog.rs`) and CLAUDE.md's architecture notes (the text input, IME in overlays). Verify with `cargo build` that no references to `TextField` remain.

## 4. Debug hooks and end-to-end checks

- [x] 4.1 Route `--send` to the focused overlay `TextInput` (control characters dropped) and add the `--- input text=… cursor=… selection=…` dump line. Teach the e2e `Dump` parser the `input` field, with a `--self-test` parser test. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`, and document the routing in the README's debug hooks section and CLAUDE.md's "Verifying GUI behaviour".
- [x] 4.2 Add e2e cases (ask the user before running, since they open windows):
  - `input-palette`: `open_command_palette`, `--send splt`, `text_input_left`, `--send i`. The input dump reads `split` with cursor 4, and the terminal screen has no `splt`.
  - `input-palette-run`: `open_command_palette`, `--send incfs`, then `picker_confirm`. The palette is closed and the dumped cell size is larger than in a run without these steps, so "Increase Font Size" ran in the tab with focus delegated to the input. The `Picker*` actions are not in `ACTIONS` yet, so register them unlabelled as `picker_up`, `picker_down`, `picker_confirm` and `picker_cancel`.
  - `input-settings-tab`: open the dialog, `settings_new`, then `settings_next_field`. The foreground field has focus, and `--send` text lands in it.
  - `input-prompt-preselected`: open the rename-window prompt in a tmux tab and `--send x`. The input holds `x`, because the pre-filled text was selected.

  Verify that `BRINDLE_E2E=1 cargo test --test e2e -- input-` passes and that the existing `settings-` cases still pass.
- [x] 4.3 Integration check: run `cargo test` and `cargo build --release`, offer to reinstall with `scripts/install.sh`, and give the user a manual checklist for what can't be driven without synthetic input: click-to-place, drag and double-click select, ctrl-c/ctrl-v with the system clipboard, and composing a character with an input method in the palette and the theme editor.
