# Design

## Context

- **`TextField`** (`picker.rs`) is a `String` with append, pop and single-line paste. Its owners feed it keystrokes: `Palette::key_down` calls `is_edit_key`/`apply_key`, and `settings_dialog::Model::key` routes keys to the editor's `focused` field index. Neither has a caret or an input handler, so there is no IME.
- **IME** already works in terminals: `TerminalElement::paint` calls `window.handle_input(&focus, ElementInputHandler::new(bounds, view))`, and `TerminalView` implements `EntityInputHandler`. Plain text arrives through `replace_text_in_range`, and `key_down` only handles keys with modifiers.
- **Keybindings resolve before `on_key_down`.** Actions bound in a context on the focused element's path win, and unbound keys bubble to ancestor contexts (`Palette`, `SettingsDialog`).
- **Focus traversal:** GPUI 0.2.2 has `FocusHandle::tab_index`/`tab_stop` and `Window::focus_next`/`focus_prev` (`examples/tab_stop.rs`).
- **Reference only:** GPUI's `examples/input.rs` (Apache-2.0) shows the pieces: a `TextElement` that shapes the line in `prepaint` and paints the selection and caret, `index_for_mouse_position` through `ShapedLine::closest_index_for_x`, and the UTF-16 conversions `EntityInputHandler` needs. Brindle is MIT OR Apache-2.0. It is read for the API shape only, and no code is copied, as with Zed's GPL crates.
- **Debug hooks:** `--send` calls `Workspace::send_to_active`, which writes bytes to the active terminal. `--action` dispatches from the focused element.
- **The settings dialog's `Model`** is pure and unit-tested. It owns the editor's text as `TextField`s.

## Goals / Non-Goals

**Goals:**
- One input used by every text entry, so palettes and the dialog behave alike.
- Editing logic that unit tests cover without a window.
- Leave the owners' navigation keys and models unchanged except where the input replaces `TextField`.

**Non-Goals:**
- Multi-line editing, undo/redo, or horizontal scrolling beyond keeping the caret in view.
- Placeholder text styling beyond what the palette already draws.
- Changing which keys the palettes and the dialog use for navigation.

## Decisions

### 1. A pure core plus a thin entity

`src/text_input.rs` holds two layers:
- `EditState { text, selection: Range<usize>, reversed, marked: Option<Range<usize>> }`, a plain struct with no GPUI dependency (this replaces `TextField`). It provides insert/replace, movement (char, word, home/end), extend-selection, deletions, `select_word_at`, single-line normalization and UTF-8 ↔ UTF-16 offset conversion. Offsets are byte indices kept on char boundaries.
- `TextInput`, a GPUI entity: `EditState`, a `FocusHandle`, the last `ShapedLine` and bounds (for mouse and IME geometry), and a drag flag. It implements `EntityInputHandler` and `Focusable`, renders a `TextInputElement`, and emits `TextInputEvent::Changed` after any edit.

Words are runs of alphanumerics and `_`; everything else is a boundary. This is enough for paths, command lines and hex values without a Unicode segmentation crate.
- *Alternative:* grow `TextField` with a cursor and keep owners feeding it keystrokes. Rejected: it can't do IME or mouse positioning without the input handler and the shaped line, which belong to an element.

### 2. Its own key context, with navigation keys left out

The element sets key context `TextInput`, and the actions are bound there:
- `left`/`right`, `home`/`end`, `ctrl-a`/`ctrl-e`, `ctrl-left`/`ctrl-right`, each with a `shift-` variant;
- `backspace`, `delete`, `ctrl-backspace`, `ctrl-w`;
- `ctrl-c`/`ctrl-shift-c`, `ctrl-x`, `ctrl-v`/`ctrl-shift-v`/`shift-insert`.

enter, escape, tab, shift-tab, up, down, ctrl-p and ctrl-n are deliberately not bound, so they bubble to `Palette` or `SettingsDialog`. ctrl-a is "go to start" (readline), not select-all, because terminal users expect it and select-all has no strong need in short fields. The actions go into `ACTIONS` unlabelled (`text_input_left`, `text_input_select_left`, and so on), so `--action` can drive them and the command palette doesn't list them.

`ctrl-shift-v` and `shift-insert` are bound to `Paste` today in the `Palette` and `SettingsDialog` contexts as well as `Terminal`. The input binds them (plus `ctrl-v`) to its own paste, and the `Palette` and `SettingsDialog` bindings and their `Paste` handlers are removed. Terminals keep `Paste`, because their context is not on an input's dispatch path.

### 3. Typed text through the input handler only

The element registers `window.handle_input(focus, ElementInputHandler::new(bounds, entity))` in paint, as `TerminalElement` does. Plain typing and IME commits arrive through `replace_text_in_range`, and composition through `replace_and_mark_text_in_range`. Marked text is drawn underlined. `bounds_for_range` uses the shaped line, so the candidate window follows the caret. There is no `on_key_down` text path, so `is_edit_key` goes away.

### 4. Owners hold entities, models stay pure

- **Palette:** `List.query` and `Prompt.text` become `Entity<TextInput>`. The palette subscribes to `Changed` and re-filters. Its root keeps the `Palette` context and focus is delegated to the input, so `Palette` actions still fire from the bubbling path. A prompt opens with its pre-filled value selected.
- **Settings dialog:** the view owns one `Entity<TextInput>` per field (name plus 23 colors). `Model` keeps plain strings: on `Changed`, the view pushes the field's text into the model (`model.set_field(ix, text)`), which runs the existing validation and preview. When the model changes a field itself (clear selection foreground), the view writes the new text back into that input. Model unit tests keep working on strings.
- **Focus order:** field focus handles get `tab_index(n).tab_stop(true)`. `SettingsNextField`/`SettingsPrevField` call `window.focus_next`/`focus_prev`, and a focus observer records the focused index in the model (for the dump and scroll-into-view). Terminals and palettes register no tab stops, so traversal stays within the dialog's fields.

### 5. Debug hooks find the focused input

`Workspace` asks its overlay for the focused `TextInput`, if any:
- `--send` with an input focused inserts the unescaped text, minus control characters, via the input's `insert` (the same path as a commit). Keys such as enter are sent with `--action`.
- `dump_active_screen` adds `--- input text=<debug> cursor=<n> selection=<a>..<b>`. The e2e `Dump` parser gains an `input` field.

### 6. Rendering

The element shapes the text once per prepaint with the dialog/palette's text style, paints the selection rectangle(s) from `x_for_index`, then the text, then a 1px-wide caret in the theme's cursor color when focused. When the text is wider than the field, the content shifts left so the caret stays visible: a per-entity scroll offset, updated in prepaint. Painting allocates only the shaped line, keeping `render`/`prepaint` cheap (GPUI re-renders the whole window on notify).

## Risks / Trade-offs

- [`focus_next` walks the window's whole tab-stop list, so another tab stop added later would join the dialog's tab cycle.] → Only text inputs register tab stops, and `focus_next` is called only from the dialog's actions. A unit test can't cover this, so the e2e tab case checks it.
- [Delegating palette focus to the input changes which element is focused, which could break the "snapshot available actions before the palette takes focus" logic.] → The snapshot is taken from the tab before the palette opens, so it doesn't depend on which palette element has focus. The new `input-palette` e2e case (task 4.2) runs a command from the palette after typing, which verifies this.
- [ctrl-c in a focused input copies instead of reaching anything else.] → Only overlays contain inputs, and they swallow the terminal's keys anyway.
- [IME behaviour varies by input method and compositor and can't be e2e-tested without synthetic input.] → Unit tests cover the marked-range bookkeeping, and the manual checklist covers a real input method (for example ibus or fcitx with a compose sequence).
- [Byte versus UTF-16 offset bugs with non-ASCII text.] → All conversions live in `EditState` and are unit-tested with multi-byte and astral characters.

## Migration Plan

None. No config or file format changes. `TextField` and `is_edit_key` are removed and their callers moved over in this change.
