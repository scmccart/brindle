# Proposal

## Why

Brindle's only text entry is `TextField` (`picker.rs`), a string that can be appended to and backspaced. It has no cursor, no selection, no mouse positioning and no IME. That's tolerable for filtering the palette or typing a hex color, but not for the next settings section, profile editing, which means fixing a typo in the middle of a path, a command line or a list of environment variables. GPUI 0.2.2 ships no text input widget (Zed's is its GPL `editor` crate), so Brindle needs its own before profile editing can be built on it.

## What Changes

- Add a single-line **text input**, a GPUI entity with its own focus, used everywhere Brindle takes typed text: the command palette and new-tab palette filter, palette prompts, and the settings dialog's theme editor fields.
- **Editing:** a caret drawn at the cursor; left/right, home/end (also ctrl-a/ctrl-e), word movement with ctrl-left/right; backspace and delete at the cursor, and ctrl-backspace/ctrl-w to delete a word back.
- **Selection:** shift with any movement key extends a selection; typing, paste, backspace and delete replace it. Clicking places the cursor, dragging selects, and double-click selects a word.
- **Clipboard:** copy, cut and paste with ctrl-c/ctrl-x/ctrl-v and with the terminal's ctrl-shift-c/ctrl-shift-v. Pasted line breaks still become spaces.
- **IME:** composition works in every text input through GPUI's input-handler path, as it already does in terminals. This reverses the settings dialog's earlier non-goal of having no IME in its fields.
- **Focus:** tab and shift-tab in the settings dialog move real keyboard focus between inputs. Keys the input doesn't use (enter, escape, up/down, tab, ctrl-p/ctrl-n) still reach the palette and the dialog, so their navigation is unchanged.
- **Debug hooks:** while a text input has focus, `--send` text goes into that input instead of the active terminal, and `--dump-screen-after` reports the focused input's text, cursor and selection. That lets e2e cases cover typing in the palette and the dialog.
- The widget is written for Brindle. GPUI's Apache-2.0 `examples/input.rs` is read for the API shape only, and no code is copied from it.

## Capabilities

### New Capabilities

- `text-input`: single-line text entry wherever Brindle asks for text: cursor movement, selection, mouse positioning, clipboard, IME, and the single-line rule for pasted text.

### Modified Capabilities

- `command-palette`: the filter and prompt text are edited with a full text input. "Backspace to edit the text" becomes full editing, and the keys the palette uses for navigation are kept out of the input.
- `settings-dialog`: keyboard operation gains in-field editing and moves real focus between fields with tab/shift-tab. The no-IME limitation is lifted.
- `command-line`: `--send` targets the focused text input when there is one, and the screen dump reports the focused input's state.

## Impact

- **Code:**
  - a new `src/text_input.rs`: a pure editing core (offsets on char boundaries, word boundaries, UTF-16 conversion, selection), with unit tests, plus the `TextInput` entity, its element and `EntityInputHandler`;
  - `picker.rs`: `TextField` is replaced by `TextInput` for the filter and the prompt, and the palette re-filters on the input's change events;
  - `settings_dialog.rs`: name and color fields become `TextInput`s, and focus moves with GPUI tab stops instead of the dialog's own field index;
  - `actions.rs`: a `TextInput` key context and its editing actions, unlabelled, so they stay out of the command palette but `--action` can drive them;
  - `main.rs` / `workspace.rs`: `--send` routing and the dump line.
- **Dependencies:** none.
- **Tests:** unit tests for the editing core; e2e cases that type into the palette and the theme editor with `--send` and assert on the dump.
