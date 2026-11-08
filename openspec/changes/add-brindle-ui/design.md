# Design

## Context

- **The surfaces today.** Brindle's overlays draw their own chrome:
  - `settings_dialog.rs` has `button()`, `field_box()`, `swatch()`; `settings_dialog/profiles.rs` has `checkbox()` and the `render_*_choice` rows.
  - `picker.rs` and `find_bar.rs` style themselves.
  - Chrome colours come from the terminal `Theme` through `theme.rs`'s derivations (`chrome_background`, `muted_foreground`, `hover_background`, `accent` → `ansi[4]`).
- **What GPUI 0.2.2 gives us:**
  - `deferred` (paints last; `with_priority` for z-order) and `anchored` (corner anchor, `SwitchAnchor` flip, `snap_to_window_with_margin`).
  - `on_mouse_down_out`, `occlude`, `FocusHandle` with `contains_focused`/`downgrade` → `WeakFocusHandle`.
  - `ManagedView` (`Focusable + EventEmitter<DismissEvent> + Render`, `window.rs:453`), `window.use_keyed_state`, `with_animation` (mount only), `App::observe_keystrokes`.
  - Keyboard clicks: a focused element with `on_click` gets `ClickEvent::Keyboard` on enter or space key-*up* (`div.rs:2184`), for whatever has focus at the key-up, with no pairing to the key-down.
- **What GPUI 0.2.2 lacks:**
  - a focus trap: `TabStopMap` is `pub(crate)`, and `focus_next` walks the whole window and wraps;
  - a layer stack, exit animations, a keyboard-vs-pointer focus signal, accessibility.
- **`TextInput`** (`src/text_input.rs`) imports only `std`, `gpui` and `crate::actions` (its 19 `TextInput*` actions, `actions!(brindle, …)`). It paints in the inherited text style and draws no box. Its default bindings are in `actions.rs::default_bindings()` under the `TextInput` context.
- **Config reload** clears and rebinds every key, so a crate that bound its own keys in `init` would lose them on the first reload.
- **The lockfile.** `Cargo.lock` must not grow third-party crates. gpui's `test-support` feature adds `backtrace` (not in the lock) and more.

## Goals / Non-Goals

**Goals:**
- Brindle can use the crate's components and primitives without the crate knowing anything about terminals, config or Brindle's actions table.
- All the decision logic is pure and unit-tested:
  - which layer acts on a press or Escape, where focus goes on Tab, and where an overlay is placed;
  - list highlight, typeahead and filtering;
  - variant and size styles.
- The GPUI glue around that logic stays thin.
- The crate stays idiomatic GPUI:
  - builder-style `RenderOnce` components;
  - stateful pieces as entities that emit events;
  - overlays as `ManagedView`s that emit `DismissEvent`;
  - actions and key contexts for keys;
  - no `Rc<RefCell>` side channels.

**Non-Goals:**
- Pixel-matching shadcn/ui. The tokens, variants and spacing scale match; fonts, shadows and animation curves only approximate.
- Changing how any Brindle surface looks or behaves. That comes in the later changes.
- Exit animations, tooltips, menus with submenus, and component-specific accessibility.

## Decisions

### Crate layout and boundary

```
ui/                         package brindle-ui, lib brindle_ui
  Cargo.toml                gpui = "0.2.2" only; version = workspace's (0.3.0)
  src/lib.rs                init(cx), default_bindings(), re-exports
  src/actions.rs            actions!(brindle_ui, [...]): TextInput*, FocusNext,
                            FocusPrev, Dismiss, ListUp/Down/First/Last, ListConfirm
  src/theme.rs              Tokens, UiTheme (Global), ActiveTheme for App
  src/style.rs              Variant/Size enums -> resolved styles (pure)
  src/icon.rs               Icon enum painted with PathBuilder
  src/primitives/
    layer.rs                LayerStack (pure) + per-window registry
    focus_scope.rs          cycle() (pure) + FocusScope element
    placement.rs            place() (pure) + Anchored element
    nav.rs                  ListNav, Typeahead (pure)
    focus_visible.rs        Modality (Keyboard | Pointer) per window
    overlay.rs              Overlay<V: ManagedView> slot (open/close lifecycle)
  src/components/
    button.rs input.rs checkbox.rs switch.rs
    dialog.rs popover.rs select.rs command.rs
  src/text_input.rs         moved from src/text_input.rs
  src/filter.rs             fuzzy_match/filter, moved from picker.rs
  examples/gallery.rs       every component; `-- --check` drives the spec's scenarios
```

- The dependency direction is `brindle → brindle-ui → gpui`, and the crate depends on nothing else.
- The public surface is `brindle_ui::{components::*, primitives::*, theme::*, TextInput, TextInputEvent, init, default_bindings}`. Internals stay `pub(crate)`.
- The lowercase scope for commits is `ui`.
- **Alternative considered:** a module inside the `brindle` crate. Rejected because it gives no enforced boundary: the compiler would let a component read `crate::settings`.

### Two kinds of component

- **Stateless components are `#[derive(IntoElement)]` + `RenderOnce` builders:** Button, Checkbox, Switch, Input and the Dialog chrome.
  - Values are controlled: the owner holds them, passes `.checked(bool)` and gets `.on_click(|new: &bool, window, cx|)`.
  - Builders implement GPUI's `Styled` for layout refinements (width, margin, flex) and their own `disabled(bool)`.
- **Anything with state of its own is an `Entity`** the owner creates, keeps and subscribes to: `TextInput`, `Select`, `Command`, and the content of every overlay.
  - `Popover` is the exception: a `RenderOnce` builder whose open state (its `Overlay` and the content entity in it) lives in keyed state, since its owner only renders it (see "Popover").
  - This matches `TextInput` today, where the owner keeps `Entity<TextInput>` and subscribes to `TextInputEvent::Changed`.
- **Focusable stateless controls take a `FocusHandle` from the owner** (`.focus(&handle)`), because a `RenderOnce` value can't keep one between frames.
  - Without a handle a control can't take keyboard focus, and pressing it doesn't move focus.
  - The owner then knows every stop, which the focus scope needs.
- **Enter and space activate on key-down, and gpui's keyboard click is ignored.** gpui raises `ClickEvent::Keyboard` on the key-up, for whatever has focus then, unpaired with the key-down. The gallery's `--check` reproduced what that does to a closing overlay: enter in a command popover (key-down) confirms, the host closes and focus returns to the button that opened it, and the key-up clicks that button and opens the popover again (the same for a command dialog). So every control runs its `on_click` on the key-down it sees itself, through `components::activates`: `key_activation` takes a first press of bare enter or space for buttons and the dialog's close `X`, and space alone for the checkbox and the switch (enter passes, as in a web form); the handler gets a `ClickEvent::Keyboard` with empty bounds, the key stops there, and a held key activates once. From gpui the controls take mouse clicks only (`accepts_click`). Select already worked this way (below). There is still no activation action.
  - **Alternative considered:** arming a per-window record on the key-down and accepting the key-up click only for the armed control. Rejected: a key-down whose key-up lands elsewhere (the opener's, once the popover has focus) leaves a stale record for the next unpaired key-up to redeem, which is the hazard again.
- **Alternatives considered:**
  - Everything as entities: heavy, and it breaks builder ergonomics for simple controls.
  - Everything `RenderOnce` with `use_keyed_state`: the state is hidden, and overlay open/close has no hook for returning focus.

### Tokens

- **Fields.** `Tokens` is a plain struct of `gpui::Hsla`:
  - pairs: `background`/`foreground`, `card`/`card_foreground`, `popover`/`popover_foreground`, `primary`/`primary_foreground`, `secondary`/`secondary_foreground`, `muted`/`muted_foreground`, `accent`/`accent_foreground`, `destructive`/`destructive_foreground`;
  - singles: `border`, `input`, `ring`, and `radius: Rems`.
- **The dialog backdrop isn't a token.** It is black at 50% opacity, as in shadcn (`bg-black/50`).
- **`destructive_foreground` is kept** even though shadcn v4 dropped it, because text on a destructive button needs a colour.
- **`chart-*` and `sidebar-*` are left out** until something needs them.
- **Neutral light and dark sets.** `Tokens::neutral_light()` / `neutral_dark()` hold shadcn's neutral oklch values, converted to sRGB once when writing them and stored as hex constants. A unit test pins a few against shadcn's published values.
- **Light and dark are separate sets.** `UiTheme` holds one token set, and the app chooses which.
- **A `dark: bool` hint.** `Tokens::dark` is false in `neutral_light()` and true in `neutral_dark()`. It is set by the token source, from the background's luminance for derived sets (`ui-tokens-from-theme`). It isn't a colour: only the switch reads it, for shadcn's dark thumb colours (`dark:data-[state=unchecked]:bg-foreground`, `dark:data-[state=checked]:bg-primary-foreground`). Without them an unchecked thumb in `background` is near black on the dark set's white-at-15% track and all but vanishes. shadcn's other `dark:` refinements (the outline button's input fill, the dimmed destructive, the switch track's `bg-input/80`) are left out.
- **Hover and active states use opacity, as shadcn does** (`bg-primary/90` → `primary.opacity(0.9)`), not extra tokens.
- **Radius steps** are derived as in shadcn: sm = radius − 4px, md = radius − 2px, lg = radius, xl = radius + 4px.
- **Access.** `UiTheme` is a GPUI `Global` read through an `ActiveTheme` trait (`cx.ui_theme()`). Replacing the global and notifying the window redraws everything (spec: "Components draw from the active tokens").
- **The font isn't a token.** Components set only text sizes and inherit the window's text style, as `TextInput` already does.
- **Alternative considered:** gpui-component's model, with per-component and hover tokens. Rejected because it is a larger surface and diverges from shadcn.

### Sizes in rems

- Components use GPUI's rem-based spacing scale (`h_9`, `px_4`, `text_sm`, …), which maps one-to-one onto the Tailwind classes shadcn's sources use. A shadcn `h-9 px-4 py-2` becomes `.h_9().px_4().py_2()`.
- Everything scales with `window.rem_size()`. Brindle's zoom today scales only the terminal font, and the chrome uses rem-based `text_xs`/`text_sm`. Whether UI zoom follows terminal zoom is for `ui-chrome` to decide; the crate only honours rems.

### Variants as enums

- `ButtonVariant { Default, Destructive, Outline, Secondary, Ghost, Link }` and `ButtonSize { Default, Sm, Lg, Icon }`.
- A pure `button_style(variant, size, &Tokens) -> ButtonStyle` (background, foreground, border, hover background, height, padding, text size) plays the role of shadcn's `cva` table.
- Tests cover every combination, so a token typo shows up as a failing test, not a wrong pixel.

### Layer stack

- **A pure `LayerStack` per window** (`Vec<LayerEntry { id, modal }>`), kept in a crate global keyed by `WindowId`. Every open overlay pushes on open and is removed on close.
- **Only the top layer acts.** A layer's `on_mouse_down_out` and its `Dismiss` action (Escape, bound in the `UiLayer` context) act only when `stack.is_top(id)`.
  - A nested menu painted outside its dialog's bounds is the top layer, so presses in it never reach the dialog's outside-press logic.
- **Decisions use the stack as it was when the event started.** Dismiss requests are queued and applied with `window.defer`. Otherwise, in one press, the menu would close first and the dialog would then see itself on top and close too.
- **Z-order.** Which layers render through `deferred` (which lifts an element above the window) is decided by kind, with `wrap_in_deferred(stack, id)` as the pure decision over the layer stack.
  - **The gpui constraint.** gpui 0.2.2 asserts that nothing calls `defer_draw` while it draws the deferred elements (`window.rs`: "cannot call defer_draw during deferred drawing"), so a `deferred` inside another's subtree panics on its first frame. Nested overlays can't each be deferred.
  - **Modal layers (dialogs) are never deferred.** The app renders them inline as the last child of its window root (Brindle's `Workspace::render_overlay` already does this for the palette), so tree order paints them above the rest of the window, and the `occlude`d backdrop, absolute at the window's origin, blocks what is beneath.
  - **Non-modal layers (a popover, a select's menu, a command popover) are deferred,** with `with_priority(depth)`, unless a non-modal layer is already open below them in the stack. A menu opened from a select in a dialog's body is therefore deferred, which is legal because the dialog isn't, and it paints above what the dialog renders after the select in tree order, such as its footer buttons, instead of under them.
  - **A non-modal layer above another non-modal layer renders inline** (with `Anchored` where it has a position), because the lower layer's view is drawn deferred. Its owner renders it inside that parent layer's view, last in the view's tree and outside any clipping container (`overflow_hidden`, a scrolled body): what the parent renders after it paints over it, and a content mask clips it. This nested-popover case is a documented limitation.
  - **Known limit.** Deferred elements paint after the whole normal tree, the root's last child included, so a non-modal layer still open when a dialog opens paints above the dialog's backdrop, and it keeps every later non-modal layer inline. Owners close popovers before opening a dialog; Brindle's one-overlay-at-a-time rule does.
  - `Overlay::render_with(place)` wraps a non-modal panel (in an `Anchored`, say) before the deferred-or-inline decision, so a deferred popover is placed between `deferred` and its panel, and an inline one is placed the same way.
- **Modal layers** cover the window with an `occlude`d backdrop. A press on the backdrop is an outside press for the panel.
- **Alternatives considered:**
  - Rely on focus alone for Escape: it works, but the outside-press problem remains.
  - gpui-component's single `Root` view with a dialog stack: it forces a wrapper view on the app and handles only dialogs.

### Overlay lifecycle: `Overlay<V: ManagedView>`

- **A slot the owner keeps** (`Option<Entity<V>>`, the `WeakFocusHandle` to restore, the `LayerId`, and the `DismissEvent` subscription).
- **`Overlay::open(entity, window, cx)`:**
  1. records `window.focused(cx)` as a weak handle;
  2. pushes the layer;
  3. focuses the entity's designated initial handle, else its first stop, else its own handle;
  4. subscribes to `DismissEvent`.
- **On `DismissEvent`, or `Overlay::close`:** it pops the layer, drops the entity, and focuses the recorded handle if it still upgrades. Otherwise it leaves focus alone for the owner.
  - **The restore is unconditional,** as the spec's "Focus returns when an overlay closes" requires, however the overlay closed. So an outside press on a non-modal layer that lands on another focusable control focuses that control during the press, and the deferred close then focuses the recorded handle (the trigger) again. The pressed control still gets its click, since click handlers don't need focus, but not focus: a second press focuses it.
  - Focus listeners compare the focus path between frames, and both moves happen before the next frame, so neither the pressed control nor the trigger sees a focus change: a terminal behind a popover sends no focus-in or focus-out report.
  - Radix's non-modal popover differs: it leaves focus where an outside press put it.
- **Outside presses and Escape don't close the overlay directly.** They dispatch `Dismiss` to the overlay's view.
  - The default handler, via a `dismiss_on_request()` helper, emits `DismissEvent`.
  - A view can handle `Dismiss` itself to stay open, for example to show an error. This is the "closes only when it agrees" requirement.
- **Placement in the tree.** A modal overlay is rendered by its owner as the last child of its window root (`.children(self.dialog.render(window, cx))` after everything else), never deeper in the tree, since nothing lifts it: tree order is its z-order. A non-modal overlay is rendered where its trigger is, and `deferred` lifts it, except one opened over another non-modal layer, which its owner renders last in the parent layer's view (the layer stack's "Z-order" bullet).
- **Multiple overlays are allowed.** Brindle's "one overlay at a time" rule (the `settings-dialog` spec) stays Workspace policy, applied in `ui-settings-dialog`.

### Focus scope

- **The scope keeps its own ordered list of stops,** because GPUI's tab order is private and window-wide.
  - An overlay view implements `FocusStops` (`fn stops(&self, cx) -> Vec<FocusHandle>`, enabled controls only, in order). The `FocusScope` element binds Tab and shift-Tab (`FocusNext`/`FocusPrev` in the `UiFocusScope` context) and moves focus with a pure `cycle(len, current: Option<usize>, dir)`.
  - When focus is on none of the stops, Tab goes to the first and shift-Tab to the last.
- **Alternatives considered:**
  - Calling `window.focus_next()` repeatedly until focus re-enters the scope: it briefly focuses outside handles, and a terminal behind the dialog would send focus-in/focus-out reports to its program.
  - A render-time registry of stops: it depends on child-view render order, which GPUI doesn't promise.

### Anchored placement

- **A crate element (`Anchored`), not GPUI's `anchored`,** because `anchored` knows only corners: it can't centre or shift along one axis.
- The element lays out its child, then calls a pure `place(trigger, size, viewport, side, align, offset, margin) -> Point`. That function flips to the opposite side when the child doesn't fit, then clamps into the viewport less the margin.
- **The trigger's bounds** are recorded each frame with `on_children_prepainted` on the trigger wrapper and kept in the popover's keyed state. An opening overlay uses the bounds from the frame that was clicked, which are current.
  - Recording runs while the frame is drawn, so it notifies nothing. When the trigger moved under an open panel (a resize, say), it asks for one more frame (`request_animation_frame`), whose render places the panel against the new bounds.

### Popover

- **`Popover` is a `RenderOnce` with keyed state** (by its id): the non-modal `Overlay` holding the content view, the trigger's bounds, and whether the popover was open at the last press on the trigger. The content closure builds a fresh view on each opening.
- **The trigger toggles through its own click.** The trigger closure gets a `PopoverTrigger` and wires its `on_click()` into the trigger's `on_click`, so a button toggles the popover on its click and on enter or space. A handler on the wrapper wouldn't do: gpui's keyboard click reaches only the focused element's own click handlers.
- **A press on the trigger is an outside press.** It asks an open popover to dismiss, and the dismissal applies before the click on release, which would then find the popover closed and open it again.
  - The wrapper records on mouse-down (bubble phase: after the panel's outside-press handler, before the deferred dismissal) whether the popover was open. The pure `trigger_action(open, mouse, pressed_open)` turns the click that follows into nothing.
  - A popover still open when its trigger is clicked (its content refused the dismissal) closes, since activating the trigger again closes it.
  - Keyboard clicks and calls from code ignore the record, which a press dragged off the trigger can leave behind.
  - **Alternative considered:** exempting the trigger's bounds from outside presses, as Radix does. It needs a press predicate through `Overlay`'s outside-press handler, and a trigger press would no longer request dismissal, against the spec's "Outside presses dismiss the topmost overlay".
- **The panel is `PopoverContent`** (shadcn's `bg-popover`, border, `rounded-md`, `w-72`, `p-4`, `shadow-md`), which the content view renders as a dialog's view renders `Dialog`. It must sit inside the layer's panel: a frame wrapped round the panel from outside would leave its padding outside the panel's bounds, where a press counts as outside.

### List navigation and typeahead

- **`ListNav { highlight, count, wrap }`** with `next`, `prev`, `first` and `last` over an `is_enabled(ix)` predicate.
- **`Typeahead { buffer, last: Instant }`** takes `push(ch, now) -> query`. Timestamps are passed in, so tests need no clock.
  - With a one-character buffer, repeating it cycles; within 1 s characters accumulate.
- **Lists:**
  - Select doesn't wrap and renders every option (lists are short), scrolling with `ScrollHandle::scroll_to_item`.
  - Command wraps (matching the palette today) and uses `gpui::list`/`ListState`, because rows have mixed heights (group headings) and palettes can be long.
- **Filtering.** `fuzzy_match`/`filter` (case-insensitive subsequence) move from `picker.rs` to `brindle_ui::filter`, and `picker.rs` imports them, so the match rule lives in one place before `ui-command-palette`.
- **Command highlights the first *enabled* match** on each change to the filter (`CommandRows::build`); a list whose matches are all disabled highlights nothing, and enter confirms nothing.

### Select

- **The trigger opens on enter and space key-down, not on gpui's keyboard click:** enter through `ListConfirm` (the trigger is in the `UiList` context), space through `on_key_down`, and a `ClickEvent::Keyboard` does nothing (`trigger_activation`). A pick on enter in the menu closes it and refocuses the trigger before the key-up, which would otherwise open the menu again; the same key-up pairing problem the controls above answer the same way.
- **Home and End are inert on a closed trigger**, as in Radix. The trigger takes `ListFirst`/`ListLast` so a list around the select doesn't move instead.
- **The menu is start-aligned under the trigger and at least as wide as it**, capped at `max-h-96` (24rem) and scrolled past that, with `Anchored` flipping it above when it doesn't fit below. shadcn's positioner sizes the menu to the room it finds instead.
- **An option press counts as pointer input** (`set_modality(Pointer)` on mouse-down), so the trigger the pick refocuses shows no ring; a pick by enter leaves the keyboard modality and the ring.

### Focus visible

- **Modality per window** (`Keyboard | Pointer`), in a crate global.
  - `init` registers `App::observe_keystrokes`, which sets `Keyboard`. gpui runs the observer for a key an action took and for one no listener stopped, but not for a key a key-down listener stopped, so the handlers that stop keys (a control's activation on enter or space, a select's typeahead and space) set `Keyboard` themselves first.
  - Every crate component's mouse-down handler sets `Pointer` before focusing.
- **The ring** shows when the control is focused and the modality is `Keyboard`. `Input` shows it whenever focused, as shadcn does.
- **No keybindings are involved.** `init` registers only observers, so it is safe to call once at startup and survives reloads.

### Actions and default bindings

- **The crate declares its actions** with `actions!(brindle_ui, [...])`, and `default_bindings() -> Vec<KeyBinding>` returns its defaults:
  - the moved `TextInput` bindings, unchanged;
  - `tab`/`shift-tab` in `UiFocusScope`;
  - `escape` in `UiLayer`;
  - `up`/`down`/`home`/`end`/`enter` and `ctrl-p`/`ctrl-n` in `UiList`.
- **`Command` binds `escape` → `Dismiss` in its own `UiCommand` context** and emits `CommandEvent::Cancelled`. So an inline command list (no overlay) still reports Escape. A hosting `ManagedView` turns `Cancelled` into `DismissEvent`. The inner context handles the key first, so the layer's own Escape binding doesn't also fire, which keeps dismissal top-layer-only.
- **Brindle chains them** into its own `default_bindings()`, so a reload restores them with the rest.
- **The `ACTIONS` table** keeps the 19 `text_input_*` config names, now pointing at `brindle_ui::TextInput*`.
- **GPUI action names** become `brindle_ui::TextInputLeft`, etc. Nothing matches on them: the only `name()` prefix check is `brindle::Find`, and logging only prints them.
- **Platform modifiers.** The bindings keep `ctrl-`, as today. `port-to-macos` switches to `secondary-` where it switches Brindle's.

### Icons

- A small `Icon` enum (`Check`, `ChevronDown`, `ChevronUp`, `X`, `Search`) painted in a `canvas` with `PathBuilder` strokes in the current text colour, sized in rems.
- **Alternatives considered:**
  - `svg()`: it needs the app's single `AssetSource`, so the crate would have to provide one for the app to chain. That couples the boundary for five glyphs.
  - Text glyphs: whether they exist depends on the font.

### Animation

- Dialog and popover entry use `with_animation`: a 150 ms fade, keyed by an open counter so each opening replays.
  - `Overlay` applies it, since it owns the open/close lifecycle: it counts its openings (`enter_animation_id(opens)`) and fades the panel, or for a modal layer the backdrop with the panel in it, as shadcn fades both. A counter rather than a fixed id, because element state lives as long as the element is rendered in consecutive frames: a slot closed and reopened between two frames would keep a fixed id's start time and not replay.
  - **No zoom.** shadcn's `zoom-in-95` isn't available: gpui 0.2.2 transforms only `svg` (`with_transformation`), not a `div`. Scaling the rem size for the panel would approximate it, but reflows and re-rasterizes its text at a new size every frame for the animation's length, so it isn't done.
- **No exit animation.** GPUI animates only on mount, so animating out needs the element kept mounted until a timer fires (a Presence primitive). That is deferred until a migration wants it.

### Testing

- **Unit tests** cover every pure piece: `LayerStack`, `cycle`, `place`, `ListNav`, `Typeahead`, `filter`, `button_style` and the token constants. `TextInput`'s existing `EditState` tests move with it.
- **gpui's `test-support` feature is deliberately rejected**, because it adds `backtrace` and other crates to `Cargo.lock`, against the lock rule.
- **GUI behaviour** (focus trap, layered dismissal, placement on screen) is checked through Brindle's e2e suite in the migration changes, as each component is wired into a surface. That is CLAUDE.md's rule for GUI checks.
- **The gallery has a `--check` mode** (`cargo run -p brindle-ui --example gallery -- --check`) that drives the Verify scenarios of tasks 4.x and 5.1–5.7 from inside the app, the way Brindle's `--action` and `--dump-screen-after` hooks do: key presses go through `Window::dispatch_keystroke` (gpui's own dispatch, bindings and all) and state is read from the components' entities and focus handles. `Window::dispatch_event` returns a type private to gpui, so key releases and pointer presses have no in-process hook; on Windows they are posted as `WM_KEYUP` and mouse messages to the gallery's own window at the bounds the components painted (messages to one window, not input to the desktop, the rule Brindle's e2e probes follow), and elsewhere those checks report `SKIP`. It prints `ok`/`FAIL`/`SKIP` per check and exits non-zero on a failure, a panic or a hang. It opens a window, so it isn't in CI; `cargo test --workspace` builds it, which keeps it compiling. Plain `cargo run -p brindle-ui --example gallery` is the interactive gallery for a person.
- **The move is checked by the existing e2e cases** that read `--- input` lines. `TextInput::describe` stays `pub` for them.

## Risks / Trade-offs

- [The behaviour of overlays and focus has no GUI check in CI until the migrations] → Keep the GPUI glue minimal and the decisions in pure functions. The gallery's `--check` covers the spec's scenarios on a developer's desktop (it found the key-up click hazard above), and each migration change adds e2e cases for the components it adopts.
- [The modality heuristic misreads mixed input, for example a key pressed while hovering] → It only affects whether a ring shows. A keystroke always makes rings visible, which is the safe side.
- [Deferred dismissals apply one tick late] → Imperceptible. The alternative is double-closing nested overlays.
- [The owner must list focus stops by hand, and a missed stop can't be tabbed to] → The gallery and each migration's e2e cases exercise Tab through every control. A debug assertion warns when the focused handle inside a scope isn't in its stops.
- [A sRGB conversion of oklch values drifts slightly from shadcn's rendering] → Pin the converted hex values in a test, and accept the sub-1% difference.
- [gpui upgrades (gpui-pre) would touch every component] → The crate isolates that churn in one place instead of every Brindle view.

## Migration Plan

1. Add the crate (empty `lib.rs`) to the workspace, set its version to match, update CLAUDE.md's crate count ("four workspace crates" → five), and commit the lockfile with only the new path entry.
2. Move `TextInput`, its actions and bindings, and `filter`, and repoint Brindle's imports. `cargo test --workspace` and the e2e cases that type into inputs must pass unchanged.
3. Add the primitives and components, then the gallery.

Each step builds and passes alone. Rolling back means reverting the change, since no config, state file or protocol is touched.
