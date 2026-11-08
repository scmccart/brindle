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
    close.
  - `FocusScope` and `FocusStops`: Tab and shift-Tab cycle through the stops a
    view lists, and never leave it.
  - `Anchored` and `place`: a box against its trigger on a side, aligned to
    its start, centre or end, flipping and clamping to stay in the window.
  - `ListNav` and `Typeahead`: roving highlight over enabled options, with or
    without wrapping, and type-to-select.
  - `focus_visible`: the per-window `Modality` (keyboard or pointer) that
    decides whether a focused control shows its ring.
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

Enter and space activate a focused control on key-down (the control's
`on_click` gets a `ClickEvent::Keyboard`); gpui's own keyboard click, which
fires on the key-up for whatever has focus then, is ignored, so a key that
confirms in an overlay and sends focus back to the opener can't click the
opener on release.

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
