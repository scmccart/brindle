# Proposal

> **Skeleton.** This holds the context from the exploration that produced `add-brindle-ui`. Flesh it out (specs, design, tasks) before implementing. **Depends on:** `add-brindle-ui`, `ui-tokens-from-theme`.

## Why

The settings dialog is the largest hand-styled surface: `settings_dialog.rs` (78K) and `settings_dialog/profiles.rs` (71K), with their own `button()`, `field_box()`, `checkbox()`, `swatch()` and the `render_*_choice` rows. Rebuilding it on `brindle-ui` removes that duplicate chrome and gives it the shared focus, dismissal and keyboard behaviour.

## What Changes

- Present the dialog through a modal `Overlay` with `Dialog` chrome, and make `SettingsDialog` a `ManagedView`. Workspace keeps the "one overlay at a time" rule.
- Replace hand-rolled controls:
  - `button()` → `Button` (Apply/Cancel/Save/Delete become default/outline/destructive variants);
  - `field_box()` + `TextInput` → `Input` (invalid state for validation errors);
  - `checkbox()` → `Checkbox`, and `Switch` where it reads better;
  - the theme/tmux/remote/engine choice rows → `Select`.
- Tab order comes from the dialog's `FocusStops` instead of `window.focus_next()`.
- Add e2e cases for what `add-brindle-ui` could only check by hand inside a real Brindle surface: Tab wrapping inside the dialog, Escape and outside-press dismissal with a select menu open, focus returning to the tab.

## Capabilities

### Modified Capabilities
- `settings-dialog`: "Keyboard and mouse operation" (Tab order, keys on focused buttons, select menus) and any requirement whose controls change kind (a choice row becoming a select). Requirements on content, validation, saving and preview are expected to stay as they are.

## Impact

- `src/settings_dialog.rs`, `src/settings_dialog/profiles.rs`, `src/workspace.rs` (how the overlay opens), `tests/e2e/cases/` (new settings cases), the `--dump-screen-after` `--- settings` line (keep its format, or update the cases that read it).

## Notes carried from exploration

- **Enter conflicts with keyboard clicks.** The `SettingsDialog` key context binds enter (and escape and tab) at key-down. GPUI's keyboard click on a focused button fires at key-up, so enter on a focused Cancel button could both apply and cancel. Decide: drop the dialog-level enter binding while a button has focus, or have buttons stop propagation.
- **Escape semantics.** The spec's cancel-on-escape maps onto `Dismiss`. The dialog handles `Dismiss` itself when it must stay open (unsaved-edit or save-failure states).
- The "One overlay at a time" requirement is Workspace policy and does not change.
