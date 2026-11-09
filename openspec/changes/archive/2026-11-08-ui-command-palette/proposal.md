# Proposal

## Why

`picker.rs`'s `Palette` (the new-tab palette, the command palette, the sessions palette, the session-ended and open-elsewhere questions, and the one-line prompt) is a hand-built filter-and-list overlay with its own list, navigation, styling and click-outside rule. `brindle-ui`'s `Command` already provides the same behaviour (the subsequence filter, wrapping navigation, ctrl-p/ctrl-n, detail and shortcut slots, `set_groups`, an empty message) on shared primitives, and `Overlay` provides the focus, dismissal and layering the palette re-implements by hand. Moving it finishes what `ui-settings-dialog` started in Workspace: no overlay is then managed outside `brindle-ui`'s `Overlay`.

## What Changes

- Rebuild the palettes on `Command` inside a modal `Overlay`, with `CommandDialog`-style chrome (a `Dialog` panel around the `Command`), as shadcn does.
- Keep `Palette` as the one host view (its `list`, `prompt`, `input` and `describe` surface, and `PaletteEvent`), so Workspace changes stay small. It holds a `Command` for list mode and an `Input` with a label and an error line for prompt mode, both inside the same overlay layer, so list to prompt keeps one layer and one focus restore.
- Per-palette outside-press rule: palettes dismiss on an outside press, except the ones that ask a question (`SessionEnded`, `SessionOpenElsewhere`, the prompt that follows them), which do not. `brindle-ui`'s `Overlay` gains a setter for that flag, since it is fixed in the constructor today. Today only the session-ended question is sticky, so the open-elsewhere question and its "New tmux session…" prompt newly ignore an outside click (user-visible).
- The palettes move from hanging under the tab bar to a centred dialog over a dimmed backdrop (shadcn's `CommandDialog`), and follow the global UI theme as the settings dialog does, instead of the active tab's profile theme. While one is open the backdrop blocks the tab strip and its `⌄` button: a press there only closes a plain palette.
- Keep `picker_*` config and `--action` names working: `picker_up/down/confirm/cancel` map to the shared `brindle-ui` list and `Dismiss` actions; `picker_kill` (ctrl-shift-delete, the sessions palette) stays an app action handled by the host on the highlighted item.
- Delete `picker.rs`'s own list, navigation, row styling and filter wiring. The `--- palette` dump lines keep their format.
- Fix what the hand-built version lacked and the shared pieces give: Tab stays inside the palette, focus returns to the tab on every way of closing, and `--- focus` reports the palette.
- Add `--key`-driven e2e cases for palette dismissal, focus return, wrap and Tab containment (the existing cases use `--action`, which bypasses the keymap).

## Capabilities

### Modified Capabilities
- `command-palette`: "Filtering and navigation" gains wrapping and the filter's own home/end; new "Dismissal and focus" requirement (Tab containment, outside-press rule per palette, focus return). Behaviour requirements on commands listed, shortcuts, running, prompt and sessions are unchanged.

## Impact

- `src/picker.rs`, `src/workspace.rs` (opening, closing and refreshing palettes; the `Overlay` enum goes), `src/actions.rs` (`Picker*` actions map to `brindle-ui` actions, config names kept), `ui/src/primitives/overlay.rs` (the outside-press setter: new public API `Overlay::set_ignore_outside_press` and `outside_press_dismisses`, under the `ui-components` requirement "Overlays can ignore outside presses", whose text still holds), `ui/src/components/command.rs` (`Command::set_empty_message`, so the sessions palette's empty message follows its list), `src/workspace.rs`'s `--- focus` line, `tests/e2e/cases/palette.rs` and the other palette cases, `CLAUDE.md` (`picker.rs` and the overlay notes).

## Notes carried from exploration

- The palette's actions are snapshotted before it takes focus (`available_actions` + `is_action_available`). That must survive the move into an `Overlay`, which moves focus on open: the snapshot is built in Workspace before the overlay opens, as now.
- The palette wraps today (`move_selection` uses `% count`). `Command` wraps too, by design.
- Row detail moves from the same line (`label  detail  [shortcut]` in the dump) to `Command`'s line under the label on screen. The dump format is unchanged.
