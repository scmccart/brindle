# Proposal

## Why

Two problems in the settings dialog's General section, found in daily use:
- The `tmux_session_end` choice field's outline collapses to a narrow box at its left edge, and the three options spill out past it. The field is built from the dialog's `field_box`, which sets `flex_1` (flex-basis 0). The General section then calls `flex_none()`, which resets grow and shrink but not the basis, so the frame stays zero-width plus padding.
- The section has no button to close the dialog. Profiles has Close and Theme has Cancel, but General has none, so the mouse can't close the dialog there. Only escape, enter, or the shortcut can.

## What Changes

- The choice field's frame takes the width of its options (flex-basis auto), so its outline encloses them.
- The General section gets a footer with a Close button at the right, like the Profiles list's. It closes the dialog, as escape does there. Changes are already saved, so nothing is reverted.
- The `settings-general` e2e case checks the outline's width in a window capture.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `settings-dialog`: the General section has a Close button.

## Impact

- `src/settings_dialog.rs` (`render_general`).
- `tests/e2e/cases/settings.rs` (`general`).
