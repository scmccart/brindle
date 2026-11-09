# Proposal

## Why

The settings dialog is the largest hand-styled surface: `settings_dialog.rs` (78K) and `settings_dialog/profiles.rs` (71K), with their own `button()`, `field_box()`, `checkbox()`, `swatch()` and the `render_*_choice` rows. Rebuilding it on `brindle-ui` removes that duplicate chrome and gives it the shared focus, dismissal and keyboard behaviour.

## What Changes

- Present the dialog through a modal `Overlay` with `Dialog` chrome, and make `SettingsDialog` a `ManagedView`. Workspace keeps the "one overlay at a time" rule.
- Replace hand-rolled controls:
  - `button()` → `Button` (Apply/Cancel/Save/Delete become default/outline/destructive variants);
  - `field_box()` + `TextInput` → `Input` (invalid state for validation errors);
  - `checkbox()` → `Checkbox` (the dialog has no switch);
  - the theme/tmux/remote/engine choice rows → `Select`.
- Tab order comes from the dialog's `FocusStops` instead of `window.focus_next()`.
- Add e2e cases for what `add-brindle-ui` could only check by hand inside a real Brindle surface: Tab wrapping inside the dialog, Escape and outside-press dismissal with a select menu open, focus returning to the tab.

## Capabilities

### Modified Capabilities
- `ui-components`: overlays can ignore outside presses; components follow shadcn/Radix behaviour (select trigger keys and pointer gestures, switch and checkbox enter handling).
- `settings-dialog`: "Keyboard and mouse operation" (Tab order, keys on focused buttons, select menus, lists and the section list as Tab stops, no up/down between editor fields) and "Sections" (a keyboard-reachable tab list), and any requirement whose controls change kind (a choice row becoming a select). Requirements on content, validation, saving and preview are expected to stay as they are.

## Impact

- `src/settings_dialog.rs`, `src/settings_dialog/profiles.rs`, `src/workspace.rs` (how the overlay opens, focus on open and close), `src/main.rs` (calls `brindle_ui::init`, and the new `--key` and `--modality` debug options), `src/actions.rs` (key contexts, `list_*` actions), `tests/e2e/` (new settings cases and harness support, e.g. the `--- focus` dump line), the `--dump-screen-after` `--- settings` line (format kept).
- `ui/`: `Overlay` options, select, switch and checkbox behaviour, `FocusRing` made public, the gallery, and the README's "Deviations from shadcn" list. `README.md` and `CLAUDE.md` keybinding and debug-hook notes, and `scripts/run-throwaway` for trying a build by hand.

## Notes carried from exploration

- **Enter conflicts with keyboard clicks.** The `SettingsDialog` key context binds enter (and escape and tab) at key-down. GPUI's keyboard click on a focused button fires at key-up, so enter on a focused Cancel button could both apply and cancel. Decide: drop the dialog-level enter binding while a button has focus, or have buttons stop propagation.
- **Escape semantics.** The spec's cancel-on-escape maps onto `Dismiss`. The dialog handles `Dismiss` itself when it must stay open (unsaved-edit or save-failure states).
- The "One overlay at a time" requirement is Workspace policy and does not change.
