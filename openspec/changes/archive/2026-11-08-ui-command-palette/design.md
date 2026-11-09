# Design

## Context

- See proposal.md for motivation. Today `Workspace` owns `overlay: Option<Overlay>` with a single `Palette` variant (settings moved to a `brindle_ui::Overlay` slot in `ui-settings-dialog`), plus `PaletteKind` (what the open palette is for), and does its own focus, dismissal and previews teardown around it.
- `picker.rs`'s `Palette` has two modes: a list (rows with label, detail and shortcut, a filter `TextInput`, a hand-rolled highlight with `% count` wrapping, `ScrollHandle`) and a prompt (a label, a `TextInput`, an error line, a running task). It emits `PaletteEvent::{Confirmed(i), Kill(i), Dismissed}`, takes `PickerUp/Down/Confirm/Cancel/Kill` actions in the `Palette` key context, and its focus is its input's.
- `brindle-ui`'s `Command` provides: `CommandItem { label, detail, shortcut, disabled }` (detail on a line under the label), groups, `with_placeholder`, `with_empty_message`, `set_groups`, `filter_input`, `rows()`/`highlighted()`, `CommandEvent::{Confirmed(i), Cancelled}`, wrapping navigation over a `gpui::list`, and an Escape binding in `UiCommand` that reports `Cancelled`. A hosting `ManagedView` turns that into `DismissEvent`.
- `Overlay` fixes outside-press behaviour at construction (`ignore_outside_press`). Today only the session-ended question ignores outside clicks (`ask_session_ended` builds it with `Palette::modal()`); every other palette, the open-elsewhere question and the prompt after it included, dismisses on one. The new rule (below) makes the open-elsewhere question and its prompt ignore them too, in one slot with the palettes that dismiss.
- The lessons of `ui-settings-dialog` apply: `brindle_ui::init` is called now, focus rings follow modality, the crate follows shadcn/cmdk strictly (deviations are listed in `ui/README.md`), and e2e cases that depend on the keymap must press keys with `--key` since `--action` bypasses it.

## Goals / Non-Goals

**Goals:**
- Same behaviour for every palette (the `command-palette` spec), plus the guarantees the overlay gives: Tab stays inside, focus returns to the tab, the dismissal rule is per palette.
- Delete the hand-built list, highlight, scrolling and styling; keep `Palette`'s outward surface so `workspace.rs` changes stay mechanical.

**Non-Goals:**
- The command list contents, shortcut rendering, tmux and session logic.
- Changing `Command`'s keys or look. Anything the palette needs that cmdk lacks stays in the host and is not added around the component's value.
- The find bar and tab strip (`ui-chrome`).

## Decisions

### Chrome: a Dialog panel around the Command (shadcn's CommandDialog)

The palette is a modal `Overlay` slot hosting a `Dialog`-chromed `Command`, centred over a dimmed backdrop. Today's palette hangs from the tab bar; centring is what shadcn's `CommandDialog` does and what the gallery's command dialog already shows. The width follows the dialog's clamp. *Alternative:* a bare popover panel anchored under the tab bar, keeping today's position. Rejected: the overlay's anchored placement is for triggers, and the palette has none.

As built (checked against shadcn's `command.tsx`): list mode is `Dialog` with `p-0`, no gap and `overflow-hidden` around the `Command`, with no visible header (shadcn's is `sr-only`) and no close `X`. shadcn's new-york `CommandDialog` defaults `showCloseButton` to true and its base preset to false; the list follows the latter, since an `X` would sit over the filter row and add a Tab stop the "Tab stays inside" rule doesn't allow. The `Command` keeps its own look: shadcn's dialog-only refinements (`h-12` input, `py-3` items) are classes on the `Command`, which the crate's entity doesn't take from its owner. The palette follows the global UI theme (`UiTheme`), as the settings dialog does, rather than the active tab's profile theme as the hand-built one did.

### One host view for both modes

`Palette` stays the single `ManagedView`: `Mode::List` holds an `Entity<Command>` (the filter input is `Command::filter_input`), `Mode::Prompt` holds an `Input`, a label, an error line and the running task. List to prompt swaps the mode inside the same view, so the overlay keeps one layer and one recorded focus to restore, with no refocus in between. `PaletteEvent` and the constructors (`list`, `prompt`, `input`, `describe`) stay, so Workspace's `PaletteKind` handling is unchanged. The host implements `FocusStops` with one stop (the active mode's input), so Tab wraps onto it.

As built:
- The host has its own focus handle, tracked on its root: the overlay keeps the view's handle for the layer's whole life, and the input changes with the mode. `--- focus palette` is "focus inside that handle".
- Every way of closing reaches Workspace as `PaletteEvent::Dismissed`: the command list's `Cancelled` in list mode, and the host's own `Dismiss` handler in prompt mode (escape and an outside press both arrive as `Dismiss`). The host never emits `DismissEvent`, so the overlay never closes behind Workspace, whose dismissal runs the ended-session and orphaned-tab rules.
- In place applies where Workspace turns a list into a prompt itself: the open-elsewhere question's "New tmux session…" (`open_prompt` swaps the mode of an open palette and sets the kind to `Prompt`). A command run from the command palette that asks for text (Rename Tab, tmux: Rename Window, tmux: Run Command) gets a palette of its own: the palette closes before the command runs, because the command must run with focus on the tab, exactly as its key would, and `Window::dispatch_action` is deferred and follows focus. Focus goes back to the tab in both cases.
- Enter in a prompt submits it, as enter in a one-field form does: `enter` is bound to `ListConfirm` in the host's `Palette` context, which `picker_confirm` names too. A list's `UiList` binding is nearer the focus, so it never reaches the host in list mode.

### The sessions palette's extras stay in the host

- `picker_kill` (ctrl-shift-delete) is bound in the host's key context and acts on `Command::highlighted()`, emitting `PaletteEvent::Kill`. It is a palette-level key, not a key around the `Command`'s value, and it does nothing in the other palettes (as today).
- After a kill the sessions list refreshes through `Command::set_groups` on the open palette (`show_sessions_palette` updates it in place, and the kind's session list with it), so the query survives. The highlight goes back to the first match, since `set_groups` refilters as typing does; keeping it would be a change to `Command`.
- "No detached sessions" is the `Command` empty message for that palette while it lists no session at all (`sessions_empty_message`); a filter matching none of the listed sessions shows the command list's own "No results found.". A refresh after a kill sets it again with the new rows (`Palette::set_rows`, through `Command::set_empty_message`, which mirrors an app swapping cmdk's `<Command.Empty>` children, so it is no deviation). Its placeholder is always "Attach a detached session".

### Outside-press rule per palette

`Overlay` gets `set_ignore_outside_press(bool)`, called by Workspace before it opens the palette (true for `SessionEnded` and `SessionOpenElsewhere`), replacing the `Palette::modal()` flag. The prompt that follows a question replaces it in place and keeps the slot's flag, so it ignores outside presses too (only the outside press is ignored: escape, a choice and the prompt's pointer-only close `X` still close it); a prompt opened on its own (Rename Tab, the tmux prompts) is a plain palette and closes on one, as the spec's "Dismissal and focus" words it. *Alternative:* two slots (a dismissable one and a question one). Rejected: it puts "one overlay at a time" across three slots.

### Actions

`PickerUp/Down/Confirm/Cancel` map to `brindle-ui`'s `ListUp/ListDown/ListConfirm` and `Dismiss`: their config names (`picker_*`) and `--action` names stay, so existing configs and e2e cases keep working, as `settings_cancel` and friends did. The default bindings come from `Command`'s own contexts, not a `Palette` context. `picker_kill` stays a `Palette` action.

### Workspace

The `Overlay` enum and `overlay: Option<Overlay>` go; Workspace keeps `palette: UiOverlay<Palette>` and `PaletteKind` beside it, and `settings`'s one-overlay rule is unchanged (opening one closes the other). Palette subscriptions (`Confirmed`, `Kill`, `Dismissed`) are held with the slot. `focused_input` and `describe` read through it. `--- focus` gets a `palette` place.

## Risks / Trade-offs

- [Row layout changes: detail sits under the label instead of beside it] → accepted, it is `Command`'s look; the `--- palette` dump text keeps `label  detail  [shortcut]`.
- [The open-elsewhere question and its "New tmux session…" prompt become sticky: a click outside, which closed them before, now leaves them open (only escape, a choice or the prompt's close button closes them)] → accepted, a user-visible change the spec asks for; README says so.
- [The modal backdrop covers the tab strip, so its `⌄` button and tabs take no press while a palette is open; the first press outside only closes a plain palette] → accepted, as with the settings dialog. A question palette ignores it.
- [Centred dialog hides more of the terminal than a top bar] → accepted; the backdrop dims, as the settings dialog does.
- [A prompt reached from a list replaces the view's mode while a layer is open, so `FocusStops` and the focused handle change under the overlay] → `multi-process-new-session` turns the open-elsewhere list into the prompt in place and submits it; `palette-prompt-focus` picks a prompted command from the command palette and checks focus in the prompt and on the tab after it (`palette-tmux-action` runs a command with no prompt).
- [`picker_*` aliases drift from the shared actions] → a keymap unit test pins that each alias maps to the same action as its shared key.
- [`Command` lacks a hook the sessions palette needs] → none found: kill acts on `highlighted()`, refresh on `set_groups`. If one turns up, it goes into `Command` with a deviation-list entry, not around it.

### The prompt's label is the dialog title

Decided with the user: in prompt mode the label ("Rename tmux window", "tmux command", the tab-rename label) is the `Dialog` title, so the prompt looks like the settings dialog and the command list, with the close X. The X is for the pointer only (no `close_focus`), so the input stays the prompt's one Tab stop; it is the deviation ui/README already lists for `Dialog`. *Alternative:* keep today's small muted label above a bare input with no title row. Rejected for consistency with the shared dialog chrome and less custom styling.
