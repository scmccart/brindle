# Proposal

> **Skeleton.** This holds the context from the exploration that produced `add-brindle-ui`. Flesh it out (specs, design, tasks) before implementing. **Depends on:** `add-brindle-ui`, `ui-tokens-from-theme`. It can land before or after `ui-settings-dialog`.

## Why

`picker.rs`'s `Palette` (the new-tab palette, the command palette, the sessions palette and the one-line prompt) is a hand-built filter-and-list overlay. `brindle-ui`'s `Command` provides the same behaviour (the same subsequence filter, wrapping navigation, ctrl-p/ctrl-n) on shared primitives.

## What Changes

- Rebuild the palettes on `Command` inside a modal `Overlay`, with `Dialog`-style chrome or a bare popover panel (decide which in design). Keep:
  - row content (`label  detail  [shortcut]`);
  - the sessions palette's `picker_kill`;
  - the prompt mode and its failure display.
- `Command` may need, beyond what `add-brindle-ui` builds:
  - a row detail and shortcut slot;
  - an extra action on the highlighted item (`picker_kill`);
  - a prompt mode, or a separate `Input` view in the same overlay.
- Delete `picker.rs`'s own list, navigation and styling. The filter already moved to `brindle_ui::filter` in `add-brindle-ui`.
- Add e2e cases for palette dismissal and focus return, if the existing `palette` cases don't cover them.

## Capabilities

### Modified Capabilities
- `command-palette`: probably none at the requirement level, since the behaviour is meant to stay the same. Confirm when fleshing out. If nothing changes, set `skip_specs: true`.

## Impact

- `src/picker.rs`, `src/workspace.rs` (opening and closing palettes), `src/actions.rs` (the `Picker*` actions may become `brindle_ui` list actions, keeping config names), the `--dump-screen-after` `--- palette` lines (keep their format; e2e cases read them).

## Notes carried from exploration

- The palette's actions are snapshotted before it takes focus (`available_actions` + `is_action_available`). That must survive the move into an `Overlay`, which moves focus on open.
- The palette wraps today (`move_selection` uses `% count`). `Command` wraps too, by design.
