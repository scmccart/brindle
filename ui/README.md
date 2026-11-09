# brindle-ui

Brindle's shared UI components on GPUI 0.2.2, modelled on
[shadcn/ui](https://ui.shadcn.com): design tokens, overlay primitives, a
component set and the single-line `TextInput`. Brindle's own surfaces (the
settings dialog, the palettes, the find bar) are rebuilt on it in later
changes; nothing in the crate is specific to Brindle.

## Boundary

The crate depends on `gpui` and nothing else. It never depends on `brindle`,
`brindle-term-core` or `alacritty_terminal`, and knows nothing of terminals,
config or the app's actions table. The dependency direction is
`brindle → brindle-ui → gpui`, and a new dependency here is a change to that
rule, not a convenience.

## What is in it

- **Tokens** (`theme.rs`): `Tokens`, shadcn's token set (background/foreground
  pairs for card, popover, primary, secondary, muted, accent and destructive,
  plus border, input, ring and the radius), held in the `UiTheme` global and
  read through `cx.ui_theme()` (`ActiveTheme`). `Tokens::neutral_light()` and
  `neutral_dark()` are shadcn's neutral sets. Mapping a terminal theme to
  tokens is the app's job.
- **Styles** (`style.rs`): variants and sizes as enums, resolved by pure
  functions (`button_style`, `input_style`, …), the role `cva` plays in
  shadcn. Sizes are in rems, so everything follows the window's text size.
- **Primitives** (`primitives/`):
  - `layer`: the per-window `LayerStack`. Escape and outside presses act on
    the top layer only, and dismissals apply after the event, so a press
    outside a popover and the dialog under it closes the popover alone.
  - `Overlay<V>`: the open/close lifecycle of a `ManagedView`: it records
    what had focus, pushes a layer, focuses the view's initial handle (else
    its first stop, else itself), wraps it in a `FocusScope`, turns Escape
    and outside presses into a `Dismiss` request, and gives focus back on
    close. A non-modal slot can block the pointer outside its view while
    open (`block_outside_pointer`, a select's menu): an outside press then
    only requests dismissal and reaches nothing beneath.
  - `FocusScope` and `FocusStops`: Tab and shift-Tab cycle through the stops a
    view lists, and never leave it.
  - `Anchored` and `place`: a box against its trigger on a side, aligned to
    its start, centre or end, flipping and clamping to stay in the window.
  - `ListNav` and `Typeahead`: roving highlight over enabled options, with or
    without wrapping, and type-to-select.
  - `focus_visible`: the per-window `Modality` (keyboard or pointer) that
    decides whether a focused control shows its ring. Only the keystroke
    observer that `brindle_ui::init` registers sets it back to keyboard
    after a pointer press, so an app that skips `init` loses its rings
    after the first click. `FocusRing` draws the ring, for an app's own
    focusable element too.
- **Components** (`components/`), in two kinds:
  - stateless `RenderOnce` builders, controlled by their owner: `Button`
    (variants default, destructive, outline, secondary, ghost, link; sizes
    default, sm, lg, icon), `Input` (a frame around a `TextInput`),
    `Checkbox`, `Switch`, the `Dialog` chrome and `PopoverContent`. A
    focusable control takes its `FocusHandle` from the owner
    (`.focus(&handle)`); without one it can't take keyboard focus.
  - entities the owner creates, keeps and subscribes to: `Select`
    (`SelectEvent::Changed`, only on a change) and `Command`
    (`CommandEvent::Confirmed` / `Cancelled`), plus `TextInput`
    (`TextInputEvent::Changed`). `Popover` is a `RenderOnce` whose open state
    lives in keyed state; its trigger closure wires `PopoverTrigger::on_click`
    into the trigger's own click.
- **`TextInput`** (`text_input.rs`) and **`filter`** (`fuzzy_match`, the
  case-insensitive subsequence match palettes use).
- **Actions** (`actions.rs`): `brindle_ui::TextInput*`, `Dismiss`,
  `FocusNext`/`FocusPrev` and `ListUp`/`ListDown`/`ListFirst`/`ListLast`/
  `ListConfirm`, bound by `default_bindings()` in the key contexts
  `TextInput`, `UiLayer`, `UiCommand`, `UiFocusScope` and `UiList`.

## Deviations from shadcn

The components follow shadcn/ui and the Radix primitives (or cmdk, for
`Command`) under it, keys and pointer included (the `ui-components` spec,
"Components follow shadcn and Radix behaviour"). Every known difference is
listed here with its reason; a difference that isn't listed is a bug. Apps
add no keys around a component: a need shadcn doesn't meet goes into the
component and onto this list, or is dropped. The select trigger's key table
is pinned by `closed_trigger_keys_match_radix` (`components/select.rs`) and
the gallery's `select-closed-trigger-keys-match-radix` check.

Everywhere:

- **Enter and space act on key-down.** A focused control's `on_click` gets
  a `ClickEvent::Keyboard` on the key-down (`activates`), and gpui's own
  keyboard click, which fires on the key-up for whatever has focus then, is
  ignored. A browser's button clicks on enter key-down but on space key-up.
  Reason: gpui's key-up click isn't paired with its key-down, so a key that
  confirms in an overlay and sends focus back to the opener would click the
  opener on release and open the overlay again. A held key activates once.
- **The focus ring is a 3px band drawn just outside the control**, not a
  box shadow. It looks the same as shadcn's `ring-[3px] ring-ring/50`.
  Reason: gpui 0.2.2's Blade and DirectX shadow shaders draw nothing for a
  shadow with no blur, and a gpui shadow also fills the area under the
  control, so it would tint transparent controls.
- **Only opening animates.** Overlays fade in over 150 ms, with no zoom or
  slide and no exit animation. Hover, focus and a switch's thumb change at
  once, with no CSS-style transitions. Reason: gpui 0.2.2 can't transform a
  `div` and has no transitions.
- **No accessibility tree.** There are no roles or `aria-*` states, so a
  screen reader sees none of these semantics. Reason: gpui 0.2.2 has no
  accessibility API (its source has no accesskit or other accessibility
  support).
- **No responsive breakpoints.** Layouts are always shadcn's `sm:` ones, for
  example a dialog's footer is always a row. Reason: these are desktop
  windows.

Button, Checkbox, Switch:

- **A checkbox has no indeterminate state** (`checked` is a `bool`). It is a
  missing feature, not a different behaviour.

Input (shadcn's is a native `<input>`, so the reference is a browser's text
field; the editing is `TextInput`'s):

- **No undo or redo** (ctrl-z, ctrl-shift-z, ctrl-y), **no ctrl-delete**
  (delete the next word) and **no context menu** (no right or middle
  button handling). These are missing features.
- **A triple click selects a word, not the whole text.** Any click count of
  2 or more selects the word under the pointer (`mouse_down`). A missing
  feature.
- **Extra keys:** ctrl-e and ctrl-shift-e (end, select to end), ctrl-w
  (delete the previous word), ctrl-shift-a, ctrl-shift-c and ctrl-shift-v
  (select all, copy and paste). Reason: these are Brindle's text-input
  bindings, moved into the crate unchanged; the ctrl-shift keys match the
  terminal's own (ctrl-shift-a is the terminal's select-all).
- **No disabled state.** `Input` has no `disabled`. A missing feature.

Dialog:

- **The close `X` is a Tab stop only when the owner gives it a handle**
  (`close_focus`) and lists that handle among its stops. In shadcn it is
  always a button in the tab order. Reason: a `RenderOnce` builder keeps no
  focus handle of its own.
- **`Overlay::ignore_outside_press`** (a "sticky" dialog) makes a press on
  the backdrop request nothing. This matches Radix's
  `onPointerDownOutside={(e) => e.preventDefault()}`, so it is a different
  API rather than a different behaviour. The backdrop still blocks the
  pointer, and Escape still dismisses.
- **A height the owner sets is capped at the window less 2rem**, as its
  width is (a `w` or `h` refinement, such as the settings dialog's 760x460,
  is held in by `max_w`/`max_h` unless the owner sets those). shadcn caps
  only the width and lets the page scroll past a tall dialog. Reason: a gpui
  window has no page to scroll.

Popover:

- **Focus goes back to the trigger however the popover closed**, an
  outside press on another control included (that control's click still
  runs, but it doesn't keep focus). Radix's non-modal popover leaves focus
  where the outside press put it. Reason: the spec's "Focus returns when an
  overlay closes" is unconditional, and a terminal behind the popover then
  sees no focus change (add-brindle-ui design, "Overlay lifecycle").
- **Tab stays inside an open popover.** Every overlay is a `FocusScope`.
  Radix's non-modal popover doesn't trap focus, so Tab can leave it, and
  focus moving outside closes it. Reason: `Overlay` wraps every view in a
  `FocusScope` (add-brindle-ui design, "Focus scope"). A non-modal variant
  without one hasn't been needed so far.

Select:

- **ctrl-p and ctrl-n** open the closed menu, as up and down do, and move
  the highlight in the open menu. Radix binds neither. Reason: they are the
  `UiList` keys every list in the crate shares, as in Brindle's palettes.
- **A menu inside a popover blocks the pointer less.** Opened over another
  non-modal layer, the menu renders inline in that layer's view
  (`Overlay`'s nesting rule), so what the popover paints after the select
  sits above the menu's pointer-blocking cover and still takes presses.
  Reason: gpui 0.2.2 forbids a deferred draw inside another.
- **The menu is always placed like a popover**: below the trigger,
  start-aligned, at least as wide as it, at most 24rem high (`max-h-96`),
  scrolled past that with a scrollbar, and flipped above when it doesn't
  fit. Radix sizes the menu to the room it finds and scrolls with its
  up/down scroll buttons, and its default `item-aligned` position overlays
  the trigger with the selected option. Reason: placement is shared with
  `Popover` (`Anchored`).
- **No groups, labels or separators in the menu**, and one trigger size
  (`h-9`). These are missing features.
- **Any primary press opens the menu**, with ctrl held too, and on the press
  for every pointer (`Select::mouse_down`). Radix opens on the press only for
  a mouse without ctrl, and opens on the release for touch and pen. Reason:
  gpui 0.2.2's mouse events carry no pointer type, and Radix's ctrl rule is
  for macOS, where ctrl-click is a context-menu click; Linux and Windows have
  no such click.
- **Typeahead on the closed trigger changes the value at once**, one
  `SelectEvent::Changed` per character that moves the selection
  (`Select::key_down`). That follows Radix, whose closed trigger calls
  `onValueChange` on each typeahead match, but an owner that writes on every
  change (the settings dialog's General section writes `config.toml`) writes
  once per matching character.

Command (shadcn's command is cmdk):

- **Filtering keeps declaration order.** Items are kept or hidden by a
  case-insensitive subsequence match (`filter`) and never re-ranked. cmdk
  scores the matches and sorts items and groups by score. Reason: the same
  match rule as Brindle's palettes, kept in one place.
- **Up and down wrap at the ends.** cmdk wraps only with `loop`. Reason: it
  matches Brindle's palettes.
- **Home and end move the filter's caret**, since the text input's bindings
  are nearer the focus. cmdk takes them to the first and last item. cmdk's
  alt- and meta-arrow jumps between groups aren't provided either.
- **Escape is the list's own `Cancelled`** (`UiCommand`), so an inline list
  reports it. cmdk leaves escape to the dialog around it. Reason: an inline
  list (in no overlay) still needs a way to be cancelled. A hosting overlay
  turns it into its own dismissal, so in a dialog the behaviour is the
  same.
- **A separator hides only beside a group the filter emptied.** cmdk hides
  every separator while the search isn't empty (unless `alwaysRender`).
  Reason: cmdk's rule drops lines between two groups that are both still
  listed (add-brindle-ui design, "Command separators").

Not verified against the upstream source here, so worth a check before
relying on them:

- Radix's switch not blocking enter. The crate now follows that reading:
  enter and space both toggle a `Switch`.
- Radix's select pointer details, which the crate follows: the 10px
  threshold (`DRAG_THRESHOLD`) for a release to count as a drag, a dragged
  release outside the content (the trigger included) closing the menu, and
  outside pointer events
  being blocked while it is open (`disableOutsidePointerEvents`).
- cmdk's `vimBindings` default. cmdk also binds ctrl-j and ctrl-k, which
  the crate doesn't.
- Whether the current shadcn `SelectContent` defaults to `popper` or
  `item-aligned`.

## Wiring an app

```rust
use brindle_ui::{Tokens, UiTheme};

Application::new().run(|cx| {
    // Once at startup: registers the keystroke observer behind
    // focus-visible, binds no keys.
    brindle_ui::init(cx);
    // The crate's keys, chained into the app's own defaults so that a
    // config reload, which clears and rebinds every key, restores them.
    cx.bind_keys(brindle_ui::default_bindings());
    // The active token set. Components read it on each render, so
    // `UiTheme::set` with another set redraws everything.
    UiTheme::set(Tokens::neutral_light(), cx);
    // …
});
```

Overlays go through `Overlay<V>`, where `V` is the app's own view: a
`ManagedView` (`Focusable + EventEmitter<DismissEvent> + Render`) that also
implements `FocusStops`, lists its focusable controls in Tab order, and
renders a `Dialog` or a `PopoverContent`. The view's own `focus_handle` must
be tracked in its tree (`div().track_focus(&self.focus)`): dismiss requests
are dispatched to it. A view that handles the `Dismiss` action itself stays
open (to show an error, say); otherwise the layer emits `DismissEvent` for it.

```rust
struct Owner { rename: Overlay<RenameDialog>, /* … */ }

// Opening: `slot` finds the overlay in the owner again when the view
// dismisses itself.
let view = cx.new(|cx| RenameDialog::new(cx));
self.rename.open(view, |owner| &mut owner.rename, window, cx);

// Rendering. A modal overlay is the LAST child of the window's root: gpui
// 0.2.2 refuses a deferred draw inside another, so modal layers are never
// deferred, and tree order is their z-order. Its backdrop is absolute at
// the window's origin and blocks what is beneath.
div()
    .size_full()
    .child(self.content(cx))
    .children(self.rename.render(window, cx))
```

Non-modal overlays (`Popover`, a `Select`'s menu) are rendered where their
trigger is and lift themselves above the window through `deferred`, a
dialog's body included. Close popovers before opening a dialog (Brindle's
one-overlay-at-a-time rule): a non-modal layer still open when a dialog opens
paints above its backdrop.

A view hosting a `Command` lists `command.focus_handle(cx)` in its stops,
turns `CommandEvent::Cancelled` into `DismissEvent`, and on `Confirmed`
closes (emits `DismissEvent`) before acting on the item.

## Gallery

```sh
cargo run -p brindle-ui --example gallery              # every component, light and dark
cargo run -p brindle-ui --example gallery -- --check   # the spec's scenarios, from inside the app
```

The gallery shows every component in every variant and size, with a dark
toggle, dialogs (one that refuses to be dismissed), popovers in each window
corner, selects and command lists inline, in a dialog and in a popover.

`--check` drives the scenarios of the `ui-components` spec (the Verify
clauses of the change's tasks 4.x and 5.1–5.7) and prints one `ok NAME`,
`FAIL NAME: why` or `SKIP NAME: why` line per check, exiting non-zero when
any failed, the app panicked or the run hung. Key presses go through gpui's
own key dispatch (`Window::dispatch_keystroke`); key releases and pointer
presses are posted as messages to the gallery's own window on Windows
(nothing is injected into the desktop) and skipped on other platforms. Both
commands open a window on the desktop, so ask before running them on a
machine someone is using.

## Testing

```sh
cargo test -p brindle-ui                           # the pure parts: layers, focus cycle, placement, lists, styles, tokens, EditState
cargo clippy -p brindle-ui --all-targets           # the gallery included
```

Every decision is a pure function with unit tests; the GPUI glue around
them is thin. GUI behaviour is checked by the gallery's `--check` on a
desktop, and by Brindle's end-to-end suite as each surface adopts a
component.
