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
  - Keyboard clicks: a focused element with `on_click` gets `ClickEvent::Keyboard` on enter or space key-up (`div.rs:2184`).
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
  examples/gallery.rs
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
  - This matches `TextInput` today, where the owner keeps `Entity<TextInput>` and subscribes to `TextInputEvent::Changed`.
- **Focusable stateless controls take a `FocusHandle` from the owner** (`.focus(&handle)`), because a `RenderOnce` value can't keep one between frames.
  - Without a handle a control can't take keyboard focus, and pressing it doesn't move focus.
  - The owner then knows every stop, which the focus scope needs.
- **Enter and space activation is GPUI's own keyboard click** (`ClickEvent::Keyboard`), so buttons need no activation action.
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
- **Light and dark aren't modelled in the struct.** `UiTheme` holds one token set, and the app chooses which.
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
- **Z-order.** Only the bottom layer renders through `deferred`, which lifts it above the window. A layer opened while another is open renders inline, with `Anchored` where it has a position, inside the open layer's view, whose `deferred` already lifts it; within that subtree, tree order gives z-order. `wrap_in_deferred(depth)` is the pure decision.
  - **The gpui constraint.** gpui 0.2.2 asserts that nothing calls `defer_draw` while it draws the deferred elements (`window.rs`: "cannot call defer_draw during deferred drawing"), so a `deferred` inside another's subtree panics on its first frame. Nested overlays can't each be deferred.
  - **The owner rule.** The owner of a nested overlay (a select's menu or a popover in a dialog) renders it inside the parent layer's view, last in that view's tree and outside any clipping container (`overflow_hidden`, a scrolled body): what the parent renders after it paints over it, and a content mask clips it. A nested modal still covers the window, since its backdrop is absolute at the window's origin.
  - `Overlay::render_with(place)` wraps a non-modal panel (in an `Anchored`, say) before the deferred-or-inline decision, so a depth-0 popover is placed between `deferred` and its panel, and a nested one is placed inline the same way.
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
- **Outside presses and Escape don't close the overlay directly.** They dispatch `Dismiss` to the overlay's view.
  - The default handler, via a `dismiss_on_request()` helper, emits `DismissEvent`.
  - A view can handle `Dismiss` itself to stay open, for example to show an error. This is the "closes only when it agrees" requirement.
- **Placement in the tree.** A bottom-layer overlay is rendered by its owner as `.children(self.dialog.render(window, cx))` anywhere in its tree, and `deferred` lifts it above everything. A nested overlay's owner renders it inside the parent layer's view, last in its tree (the layer stack's "Z-order" bullet).
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

### List navigation and typeahead

- **`ListNav { highlight, count, wrap }`** with `next`, `prev`, `first` and `last` over an `is_enabled(ix)` predicate.
- **`Typeahead { buffer, last: Instant }`** takes `push(ch, now) -> query`. Timestamps are passed in, so tests need no clock.
  - With a one-character buffer, repeating it cycles; within 1 s characters accumulate.
- **Lists:**
  - Select doesn't wrap and renders every option (lists are short), scrolling with `ScrollHandle::scroll_to_item`.
  - Command wraps (matching the palette today) and uses `gpui::list`/`ListState`, because rows have mixed heights (group headings) and palettes can be long.
- **Filtering.** `fuzzy_match`/`filter` (case-insensitive subsequence) move from `picker.rs` to `brindle_ui::filter`, and `picker.rs` imports them, so the match rule lives in one place before `ui-command-palette`.

### Focus visible

- **Modality per window** (`Keyboard | Pointer`), in a crate global.
  - `init` registers `App::observe_keystrokes`, which sets `Keyboard`.
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

- Dialog and popover entry use `with_animation` (150 ms fade, and for dialogs a 95%→100% zoom), keyed by an open counter so each opening replays.
- **No exit animation.** GPUI animates only on mount, so animating out needs the element kept mounted until a timer fires (a Presence primitive). That is deferred until a migration wants it.

### Testing

- **Unit tests** cover every pure piece: `LayerStack`, `cycle`, `place`, `ListNav`, `Typeahead`, `filter`, `button_style` and the token constants. `TextInput`'s existing `EditState` tests move with it.
- **gpui's `test-support` feature is deliberately rejected**, because it adds `backtrace` and other crates to `Cargo.lock`, against the lock rule.
- **GUI behaviour** (focus trap, layered dismissal, placement on screen) is checked through Brindle's e2e suite in the migration changes, as each component is wired into a surface. That is CLAUDE.md's rule for GUI checks.
- **`gallery` is for manual review only.** `cargo test --workspace` builds it, which keeps it compiling.
- **The move is checked by the existing e2e cases** that read `--- input` lines. `TextInput::describe` stays `pub` for them.

## Risks / Trade-offs

- [The behaviour of overlays and focus has no automated GUI check until the migrations] → Keep the GPUI glue minimal and the decisions in pure functions. Each migration change adds e2e cases for the components it adopts.
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
