# Design

## Context

- See proposal.md for motivation. The dialog is two files, `settings_dialog.rs` (model, theme list and editor, general section, render) and `settings_dialog/profiles.rs` (profile list and editor, with `open_choice` state for its inline menus).
- Today it is `Overlay::Settings` in `workspace.rs`: Workspace owns `overlay: Option<Overlay>` (palettes and settings), focuses the dialog itself, and `dismiss_overlay` refocuses the active tab. Tab inside the dialog is `next_field`/`prev_field` actions that step `Editor::focused`, and `sync_focus_from_window` copies a clicked input's focus back into the model.
- `brindle-ui` provides `Overlay<V: ManagedView + FocusStops>` (modal slot, focus restore, `FocusScope`), `Dialog`, `Button`, `Input`, `Checkbox`, `Switch` and `Select`. An overlay turns Escape *and* an outside press into the same `Dismiss` request, and a view that handles `Dismiss` itself stays open.
- Controls activate on enter/space **key-down** and stop the key there (`components::activates`), and ignore gpui's key-up keyboard click.
- Palettes still use Workspace's own `overlay` enum until `ui-command-palette`.

## Goals / Non-Goals

**Goals:**
- Same behaviour as the `settings-dialog` spec, except where the delta changes it; the model (`Model`, `Editor`, `ProfileEditor`, `Outcome`) stays pure and keeps its tests.
- Delete the dialog's duplicate chrome: `button()`, `field_box()`, `checkbox()`, the `render_*_choice` rows and the `open_choice` bookkeeping.

**Non-Goals:**
- Content, validation, saving and preview semantics. The list rows, the section list and the colour swatches stay hand-drawn (no component for them yet); they take their colours from the `UiTheme` tokens.
- Palettes, find bar and tab strip (`ui-command-palette`, `ui-chrome`).

## Decisions

### Workspace owns an `Overlay<SettingsDialog>` slot

`Overlay::Settings` leaves the enum; Workspace gets `settings: ui::Overlay<SettingsDialog>` (modal), renders it last in the window tree, and subscribes to the dialog's events as now. "One overlay at a time" stays Workspace policy: opening a palette closes the settings slot and opening settings closes the palette. `SettingsEvent::Close` becomes `DismissEvent`, so the slot restores focus to the tab that had it, and closing also ends the theme and profile previews (the current `close_overlay` work). `focused_input` and `describe` read through the slot.

*Alternative:* put settings inside the existing enum and call `Overlay::open` there. Rejected: the slot needs its own type parameter and the enum would hold a non-uniform variant for one change.

### Outside presses are ignored, through a new overlay option

The spec says a stray click never closes the dialog (it holds previews and edits). The overlay can't tell a press from Escape once it is a `Dismiss`, so the dialog can't filter them. `Overlay::modal()` gains `.ignore_outside_press()`, which skips the `on_mouse_down_out` request for that layer; the backdrop still occludes. This is a small addition to `brindle-ui` with its own scenarios in `ui-components` and a unit test on the pure decision.

*Alternatives:* set a flag in the dialog on press and swallow the next `Dismiss` (racy, the hazard the crate's activation design avoids); accept click-outside as cancel (changes a stated requirement and loses edits).

### Dismiss is the dialog's cancel

Escape arrives as `Dismiss`. The dialog handles it: an open choice menu is the topmost layer and takes it first (layers dismiss top-down); otherwise `Dismiss` runs today's `cancel` (discard in an editor, cancel in a list, close in General). Emitting `DismissEvent` happens only when the model's outcome is `Close`/`Applied`. The `SettingsDialog` context keeps enter, up/down, alt-up/down, section and theme-list keys; escape and tab leave it. Up/down act only in the lists: the old stepping between an editor's fields goes, since tab does that job and up/down there would be keys added around the text inputs and selects.

### Tab order from `FocusStops`

`SettingsDialog` implements `FocusStops` per section and mode: the stops are the section list (one stop), then the mode's fields (or, in the theme and profile lists, the list itself as one stop), then its footer buttons, then the close button.

The section list is app code shaped like Radix's vertical `Tabs` with automatic activation: one stop (`sections_handle`, key context `SettingsSections`) whose up/down (wrapping, Radix's `loop` default) and home/end change the section at once through the same `Model::select_section` a click uses, so an unsaved editor still holds the section and leaving Theme still drops its preview. Focus stays on the list while sections change (`place_focus` leaves it alone), and the shown section draws the ring. A press on a section focuses the list, as a press on a tab focuses it. Enter on the list does nothing. ctrl-tab, ctrl-shift-tab and ctrl-pageup/down stay as dialog-wide shortcuts; an e2e case presses each from a select, a list, a text field and a button. It stays in the dialog rather than becoming a brindle-ui `Tabs`, since it is the only tab list so far.

Each list is one stop with its own handle (`list_handle`) and the `SettingsList` key context, where up/down, ctrl-p/ctrl-n, home and end move the selection; those keys left the `SettingsDialog` context, so they do nothing on a footer button. The list's frame draws brindle-ui's ring (`FocusRing`, made public for this) by the same focus-visible rule as the components, outside the scrolling list so it isn't clipped. A list takes focus when its view opens.

Focus rings follow brindle-ui's focus-visible modality, which a pointer press on a control sets to pointer and only the crate's keystroke observer sets back to keyboard. Brindle never called `brindle_ui::init`, so after the first press no ring came back; `main` now calls it. A click on a section counts as pointer input too. The e2e cases check this with two debug steps: `--key` (gpui's `Window::dispatch_keystroke`, so bindings, key listeners and keystroke observers run as for a real key) and `--modality`, plus `stop=` and `modality=` on the `--- focus` dump line, and compare window captures taken with and without a ring. The dialog keeps one `FocusHandle` per field (the `TextInput`s' and the `Select`s' own) and per button. `Editor::focused` and `next_field`/`prev_field` go; the `--- input` and `field=` dump values are derived from which stop is focused, so their formats are unchanged. `sync_focus_from_window` is removed since focus is the state.

### Controls

- `button()` → `Button`: Apply/Save/Close `Default`, Cancel/Discard/New/Edit `Outline`, Delete `Destructive`, each with a focus handle.
- `field_box()` + `TextInput` → `Input`, `.invalid(...)` for bad colours, name and numeric fields; swatches stay.
- `checkbox()` → `Checkbox` (the profile default toggle). `Switch` is not used: the existing wording is checkbox-like and the spec says checkbox.
- Choice rows → `Select`: General session-end, profile theme, tmux mode, remote kind and engine. The owner subscribes to `SelectEvent::Changed` and applies it. `open_choice` and the inline menus are deleted. **No keys are added around a select**: left/right do nothing on it, as in Radix (the old `SettingsChoice` context and the `settings_choice_next/prev` actions are removed). Keyboard tests pick through the menu (open, down, enter).
- Enter on a focused button or the close X is passed on by the dialog's confirm handler. gpui runs key bindings before key-down listeners, so the control cannot stop the key first. Unit tests cover the binding and the pass-through, and the `settings-enter-on-button` e2e case presses enter through `--key` on a focused Discard and Cancel. A real platform keypress is still a manual check.

### Stick to shadcn

The crate's components copy shadcn/ui's behaviour (`ui-components`: "Components follow shadcn and Radix behaviour"). To keep it so:
- `ui/README.md` has a "Deviations from shadcn" list with a reason per entry; a difference not on it is a defect.
- Each component's keyboard table is pinned by a pure unit test (for the select trigger: `typed_key`/activation tables) and a gallery `-- --check` case, so a change to a key's behaviour fails a test.
- Apps add no keys around a component. A need shadcn doesn't meet goes into the component and onto the deviation list, or is dropped. The dialog's own keys (list keys, section list, ctrl-tab and ctrl-pageup/down) are dialog-level, not keys around a component's value.
- Following this, the select's pointer behaviour was made Radix's: the trigger opens on press, the menu picks on release (a release after a move of over 10px closes it), and a press outside an open menu is caught by a transparent cover (`Overlay::block_outside_pointer`) so it only closes the menu. The cover has one known limit, listed in `ui/README.md`: a select inside a popover renders inline in it, so what the popover paints after the select still takes presses. The switch now toggles on enter as well as space, and the checkbox still ignores enter.
- The deviation list gained entries for each remaining difference (including the dialog height clamp this change added), with reasons.

### Sizing and tokens

The panel keeps its 760x460 size through `Dialog`'s `Styled` refinement; `Dialog` supplies title ("Settings"), close X and chrome. Hand-drawn parts read `cx.ui_theme()` tokens (from `ui-tokens-from-theme`) instead of `Theme::chrome_*`, so they follow the preview.

## Risks / Trade-offs

- [`Dialog`'s max width is 32rem, narrower than 760px] → set the width by refinement and check the gallery; if `max_w` wins, add a width override to `Dialog` in this change.
- [Select menus inside a dialog with `Dismiss` handling] → the topmost-layer rule already covers Escape; an e2e case checks Escape, outside press and focus return with a menu open.
- [`--dump-screen-after` `--- settings` line] → derive the same fields; the existing e2e cases read it and must pass unchanged.
- [Left/right no longer change a choice field, a loss for keyboard users of the old inline rows] → intended: it is what shadcn does, and a menu is two keys away (open, pick).
