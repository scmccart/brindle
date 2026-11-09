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
    The trigger may be a zero-size rectangle (a point, for a context menu).
    `available_height` is the tallest a box may be beside its trigger (the
    larger room above or below it, or the window's height for a side
    placement, less the 8px `EDGE_MARGIN`), which caps a menu's height.
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
    default, sm, lg, icon, icon-sm, icon-xs; the icon sizes are squares of
    2.25, 2 and 1.5rem, and the icon inside is 1rem, 0.75rem in icon-xs),
    `Input` (a frame around a `TextInput`), `Checkbox`, `Switch`, `Toggle`,
    the `Dialog` chrome and `PopoverContent`. A focusable control takes its
    `FocusHandle` from the owner (`.focus(&handle)`); without one it can't
    take keyboard focus.
    - `Toggle` (`Toggle::new(id, "Aa")` or `Toggle::icon_only(id, icon)`):
      shadcn's toggle, a button with a `pressed` state the owner holds.
      Variants default and outline, sizes default, sm and lg (`h-9`, `h-8`,
      `h-10`, each also the min width, so an icon or one letter makes a
      square). A primary click, or enter or space while focused, calls
      `on_click(&new_state, …)`; pressed, it is the accent pair, unpressed a
      transparent control that hovers muted (`toggle_style`). Disabled, it
      is drawn at half opacity, asks nothing and takes no focus. Without a
      focus handle a press on it moves focus nowhere, not even to an
      ancestor that tracks focus (a find bar's query keeps it).
    - `Tooltip` (`Tooltip::new("New tab").shortcut("ctrl-shift-t")`): a
      label and an optional shortcut as `Kbd` keycaps, drawn inverted
      (`foreground` behind `background` text, `text-xs`, `tooltip_style`).
      `TooltipFor` puts one on an element: `.tooltip_for(label, shortcut)`
      with fixed strings, or `.tooltip_with(|window, cx| Tooltip::new(..))`
      to work the text out (a shortcut from the keymap) when it is shown.
      Either way the view is built only when it shows. It works on any
      stateful element (`div().id(..)`) and on `Button` and `Toggle`, which
      drop it while disabled. gpui's `tooltip` shows it after a 500 ms rest
      and hides it when the pointer leaves, on a press and on a scroll; the
      shown view hides itself on Escape (`intercept_keystrokes`, which sees
      the key before any binding and lets it go on). A `Tooltip` also
      renders in place, as the gallery shows its look.
    - `TabList` (`TabList::new(id).variant(..).active(ix).children(triggers)`)
      and `TabTrigger` (`TabTrigger::new(id, label)`): shadcn's tab list and
      triggers, with what a window's tab strip adds. The owner keeps the
      triggers and which one is active and is asked to change them:
      `on_activate(&TabActivate { index, by })` (`by` is the pointer or the
      keyboard, so the owner decides where focus goes), `on_close(&index)`
      and `on_move(&TabMove { from, to })`; `on_trigger_bounds` reports where
      the triggers were laid out. Variants `Default` (shadcn's pill),
      `Line` and `Attached` (a tab strip: the active trigger is drawn on a
      surface with rounded top corners and a 2px primary line on top, in
      the trigger's own `surface` and `text_color` when it has them, else
      the background and foreground tokens; `tab_style`). The list is one
      keyboard stop through the handle `.focus(&handle)` gives it, with
      focus resting on the active trigger: left and right activate the
      previous and next enabled trigger, wrapping, home/PageUp and
      end/PageDown the first and last (`tab_key`, `tab_target`). A primary
      press asks to activate and moves no focus. A trigger takes a leading
      element (an icon), trailing adornments (status dots), a tooltip,
      `disabled` (half opacity, skipped, asks nothing) and `closable`: a
      ghost `IconXs` close button shown on the active trigger and on a
      hovered one, whose press asks to close and doesn't activate; a middle
      press on a closable trigger asks to close it too. `trigger_width`
      (`TabWidth::new(min, max)`) lets triggers shrink to the minimum and
      grow with their label to the maximum, cutting it with an ellipsis.
      With `on_move`, a trigger dragged onto another of the same list
      (`DraggedTab`, previewed as a pill with its label) asks to move there,
      and a 2px primary bar marks the side it will land on (`drop_side`).

    Two of the builders are not controls:
    - `Separator` (`Separator::horizontal()` / `vertical()`): a one-pixel
      rule in the border colour that takes no focus and no pointer events.
    - `Kbd` (`Kbd::new("ctrl-shift-t")`): keycaps from a keymap string, one
      per key with the names unchanged. Strokes separated by a space
      (`ctrl-k ctrl-c`) are groups with a wider gap between them. `keycaps`
      does the splitting, `.colors(background, foreground)` replaces the
      muted tokens (an inverted tooltip). It prints what the keymap prints;
      mapping `cmd` or `super` per platform is the keymap's job.
  - entities the owner creates, keeps and subscribes to: `Select`
    (`SelectEvent::Changed`, only on a change) and `Command`
    (`CommandEvent::Confirmed` / `Cancelled`), plus `TextInput`
    (`TextInputEvent::Changed`). `Popover` is a `RenderOnce` whose open state
    lives in keyed state; its trigger closure wires `PopoverTrigger::on_click`
    into the trigger's own click.
    - Menus: `Menu` is the content both openers show, a list of `MenuEntry`
      (`Item(MenuItem)` with a label, an optional `description` line under
      it in the muted colour, an optional `shortcut` drawn as `Kbd` keycaps
      and `disabled`; or `Separator`). It emits `MenuEvent::Selected(index)`
      (separators counted); Escape and an outside press only close it. It
      blocks the pointer outside it, has its height capped at the room
      beside its anchor and scrolls, keeping the highlight in view. The
      highlight starts on nothing; up/down (and ctrl-p/ctrl-n) move over the
      enabled items without wrapping, home/PageUp and end/PageDown go to the
      ends, typing matches labels (`MenuNav`), and enter or space selects. A
      primary release on an enabled item selects it; the release ending the
      press that opened the menu selects only after a move of more than
      10px (`menu_item_release`), so press, drag and release is one gesture.
      Focus goes back to what had it when the menu opened.
    - `ContextMenu`: the owner renders it anywhere (it takes no room) and
      opens it from a target's secondary press with
      `.on_context_menu(|event, window, cx| …)` (`ContextMenuTarget`) and
      `ContextMenu::open_on_press(event, entries, …)`, or at a point from a
      key or code with `open(at, entries, …)`. The menu's corner goes at the
      pointer, flipping to its left near the window's right edge and shifted
      up near the bottom.
    - `DropdownMenu::new(trigger, entries)`: renders its trigger from the
      closure (a `Button` with no `on_click`, and a focus handle for the
      keys) and opens the menu below it, start-aligned and 4px away, on a
      primary press or enter, space or down from the focused trigger
      (`trigger_key_opens`). A second press on the trigger closes it.
      `set_disabled(true)` makes it open nothing (draw the trigger disabled
      too).
    - An app that keeps its own overlay slot for menus (Brindle's workspace)
      creates a `Menu` with `Menu::new(entries, MenuAnchor::at(point) or
      MenuAnchor::below(bounds), press, cx)`, opens it in a `menu_overlay()`
      slot and draws it with `render_menu`.
- **Icons** (`icon.rs`): lucide's shapes drawn from polylines, circles and
  rounded rectangles (`Stroke::Rect`) in the text colour, sized in rems:
  `Check`, `ChevronDown`, `ChevronUp`, `X`, `Search`, `Plus`, `Minus`,
  `Square`, `Copy`, `AppWindow`, `ArrowUp`, `ArrowDown`, `Columns2` and
  `Rows2`. Strokes have butt caps and miter joins, since gpui doesn't expose
  lucide's round caps.
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
the gallery's `select-closed-trigger-keys-match-radix` check; the dropdown
trigger's by `trigger_keys_match_radix` (`components/dropdown_menu.rs`) and
the gallery's `dropdown-enter-space-down-open-below-the-trigger-other-keys-do-not`
check; the tab list's by `keys_map_as_radixs_roving_focus`
(`components/tabs.rs`) and the gallery's
`tabs-home-end-pageup-pagedown-go-to-the-ends-other-keys-do-nothing` check.

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

Button, Checkbox, Switch, Toggle:

- **A checkbox has no indeterminate state** (`checked` is a `bool`). It is a
  missing feature, not a different behaviour.
- **A toggle without a focus handle is not a Tab stop, and a press on it
  moves focus nowhere.** Radix's toggle is a `<button>`, always in the tab
  order, and a press focuses it. Reason: a `RenderOnce` builder keeps no
  handle of its own (as for Dialog's close `X`), and a find bar's option
  toggles must leave focus in its query. With a handle it behaves as
  Radix's. A button without a handle doesn't take focus either, but its
  press still bubbles to an ancestor that tracks focus.
- **A toggle has no `aria-invalid` state**, so shadcn's invalid ring and
  border aren't drawn. A missing feature.

Tooltip (shadcn's is Radix's Tooltip; ours is gpui's `tooltip` with the
crate's look):

- **No delay group, and a fixed 500 ms delay.** The delay is gpui's
  (`TOOLTIP_SHOW_DELAY`), with no skip-delay when the pointer moves from
  one trigger to the next. shadcn's `TooltipProvider` sets
  `delayDuration={0}` (Radix's default is 700 ms) and Radix skips the delay
  for 300 ms after a tooltip closed. Reason: gpui 0.2.2 doesn't expose the
  delay or the shown tooltip's state.
- **Placed at the pointer, not beside the trigger.** gpui puts the tooltip
  1px below and right of the pointer where it rested, flipped to the other
  side of the pointer at the window's edges. Radix centres it on a side of
  the trigger (`top`, `sideOffset` 0 in shadcn), with collision handling.
  Reason: gpui places tooltips itself and gives the view no anchor.
- **No arrow.** shadcn draws `TooltipPrimitive.Arrow`. Reason: the
  placement is the pointer's, not a side of the trigger, so an arrow would
  point at nothing in particular, and gpui 0.2.2 can't rotate a `div`.
- **Hover only: focusing the trigger from the keyboard shows nothing.**
  Radix opens a tooltip on keyboard focus too. Reason: gpui's tooltip
  follows the pointer alone.
- **The pointer can't move onto the tooltip to keep it open.** Radix keeps
  hoverable content open (`disableHoverableContent` false). Reason: the
  spec says a tooltip takes no pointer events (gpui's `hoverable_tooltip`
  would make it hoverable).
- **Any wheel event over the window hides it** (gpui's rule). Radix closes
  a tooltip only when an element containing its trigger scrolls.
- **One line, no wrapping.** shadcn's content is `text-balance` and wraps
  at the page's width. Reason: gpui lays a tooltip out at its smallest
  size, so wrapping text would break at every word.
- **No open fade or zoom.** shadcn animates `fade-in-0 zoom-in-95`. Reason:
  gpui shows the tooltip view as is (see "Only opening animates").

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
- **`Overlay::ignore_outside_press`** (a "sticky" dialog), or
  `set_ignore_outside_press(true)` on a slot whose views differ (Brindle's
  palettes: only the ones that ask a question are sticky), makes a press on
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

Menus (`ContextMenu` and `DropdownMenu`; shadcn's are Radix's ContextMenu and
DropdownMenu, both over Radix's Menu, modal by default). The pointer
gesture and keys were compared against Radix's source (`packages/react/menu`,
`dropdown-menu` and `context-menu` in radix-ui/primitives) and shadcn's
new-york-v4 `dropdown-menu` and `context-menu`, read while implementing the
ui-chrome change. What matches: down, PageUp and Home highlight the first
enabled item from none, up, PageDown and End the last; up and down don't
wrap (`loop` is off by default); enter and space select; a space during a
typeahead search is part of the search; Tab does nothing; the pointer
highlights an enabled item it moves over and clears the highlight over a
disabled one or on leaving; a primary release on an enabled item selects
it wherever the press came; a release elsewhere does nothing (so a dragged
release outside keeps the menu open, unlike `Select`, whose Radix
counterpart closes there); outside pointer events are blocked; the
dropdown's trigger opens on a primary press, on enter or space (which
toggle in Radix, and the trigger only has focus while closed) and on down;
it sits 4px below the trigger (shadcn's `sideOffset`); the height is capped
at the available height and scrolls. The differences:

- **The release that ends the opening press selects only after a move of
  more than 10px.** Radix's item selects on any pointer-up that didn't
  follow a press on that item (`isPointerDownRef`), with no threshold.
  Reason: the spec takes `Select`'s gesture rule; and with our context
  menu's corner under the pointer, a release after a slight move would
  otherwise land on the first item and select it.
- **The dropdown is aligned to the trigger's start.** Radix's popper
  content defaults to `align="center"`, and shadcn's `DropdownMenuContent`
  passes only `sideOffset={4}`, so its default menu is centred under the
  trigger (a call site opts into `align="start"`). Reason: the spec
  (anchored below the trigger, aligned to its start).
- **The context menu's corner is at the pointer** (`sideOffset` 0). Radix's
  `ContextMenuContent` uses `side="right" sideOffset={2} align="start"`, so
  its corner is 2px right of the pointer. Reason: the spec's "corner at
  (200, 40)"; the threshold above covers what the 2px did.
- **Opening by key highlights nothing.** Radix's menu focuses the first
  enabled item when the trigger was opened from the keyboard
  (`isUsingKeyboardRef`), and nothing when opened by the pointer. Reason:
  the spec ("opening it moves the highlight to nothing; the first down key
  highlights the first enabled item"), the same for both ways in.
- **Focus always goes back to what had it when the menu opened.** Radix's
  dropdown focuses its trigger on close, except after a right-click outside
  (or when non-modal); its context menu returns focus as ours does. Reason:
  the spec's "Focus returns when an overlay closes" is unconditional; a
  trigger with a focus handle gets focus back either way, and one without
  (Brindle's `⌄`) leaves it with the terminal.
- **A ctrl-press opens the dropdown, on the press for every pointer.**
  Radix's trigger ignores a press with ctrl and opens on pointer-down only
  for a mouse. Reason: as for `Select` (no pointer type in gpui's events;
  ctrl-click is a macOS context-menu click).
- **The context menu opens on the secondary press on every platform.**
  Radix opens on the browser's `contextmenu` event, which comes on the
  press on Linux and macOS and on the release on Windows, and also from the
  keyboard (the Menu key, shift-F10). Reason: gpui has no context-menu
  event; the spec says press. No key opens it: `ContextMenu::open(at, …)`
  is there for an app's own binding.
- **No long press.** Radix opens a context menu after a 700 ms touch or pen
  press. Reason: gpui's events carry no pointer type.
- **ctrl-p and ctrl-n move the highlight**, as in every `UiList`. Radix
  binds neither.
- **Shortcuts are `Kbd` keycaps**, not shadcn's `DropdownMenuShortcut` text
  (`ml-auto text-xs tracking-widest text-muted-foreground`). Reason: the
  ui-chrome design draws every shortcut with `Kbd`. In the neutral tokens
  `muted` and `accent` are the same colour, so on the highlighted row the
  keycaps' boxes don't show (their text does).
- **Items may carry a description line** under the label. An addition: a
  Radix item is whatever its children are. Typeahead matches labels only;
  Radix matches an item's `textValue`, which defaults to its text content
  (description and shortcut included), by prefix, so the two differ only
  for a query longer than the label.
- **The height cap is the larger room above or below the trigger** (for the
  context menu, the window's height), less the 8px edge margin. Radix's
  `--radix-*-content-available-height` comes from floating-ui's `size`
  middleware with no collision padding.
- **The dropdown trigger's press stops there**, so an ancestor's handlers
  (one that tracks focus, a window menu) don't run; the context target's
  secondary press too.
- **No submenus, checkbox or radio items, labels, groups, `inset` or
  `destructive` items, and no open or close animation beyond the fade** (see
  "Only opening animates"). Missing features.

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

Tabs (`TabList` and `TabTrigger`; shadcn's are Radix's Tabs over its
RovingFocusGroup). Compared against Radix's `packages/react/tabs` source and
shadcn's new-york-v4 `tabs`, read while implementing the ui-chrome change.
What matches: the list is one Tab stop, entered on the active trigger; left
and right move to the previous and next enabled trigger and activate it
(automatic activation, Radix's default), wrapping at the ends (`loop`
defaults to true on Radix's `TabsList`); home and PageUp go to the first,
end and PageDown to the last; up and down do nothing in the horizontal list;
a key with ctrl, alt, shift or super held is left alone; a disabled trigger
is skipped and a press on it does nothing; a non-primary press activates
nothing; the pill and line looks, sizes, colours (dark ones included) and the
focus ring are shadcn's. The differences, mostly what a window's tab strip
needs:

- **A primary press activates but moves no focus**, and no press anywhere on
  the list does. Radix's trigger focuses itself on the press, then
  activates. Reason: the owner decides where focus goes (`TabActivate::by`):
  Brindle's strip keeps it in the tab's terminal, and its list takes focus
  only from the keyboard.
- **Focus follows activation.** The list has one focus handle and focus
  rests on whichever trigger the owner made active, so an owner that ignores
  an arrow's request leaves focus where it was. Radix moves focus to the
  next trigger whatever the owner does with the value. Reason: a
  `RenderOnce` builder keeps no handle per trigger.
- **Enter, space and Escape are left to the owner.** Radix's trigger
  activates on enter and space, which in automatic mode changes nothing,
  since the focused trigger is already the active one. Reason: the owner can
  use them (Brindle's strip sends focus back to the terminal on them).
- **A ctrl-press activates.** Radix's trigger ignores a press with ctrl.
  Reason: as for `Select` (ctrl-click is a macOS context-menu click).
- **Closable triggers** (an addition): a ghost `IconXs` close button at the
  trigger's end, shown on the active trigger and on a hovered one, and
  keeping its room when hidden so a trigger's width doesn't change under the
  pointer. Its press asks the owner to close the trigger and doesn't
  activate it, start a drag or move focus. Reason: a tab strip closes tabs.
- **A middle press closes a closable trigger, on the press.** Radix's
  trigger does nothing on a middle press; a browser's tab strip closes on
  the middle release (`auxclick`). Reason: tab strips close on a middle
  click, and Brindle's strip has always done it on the press.
- **Adornments and a tooltip** (additions): a leading element (an icon),
  trailing adornments (status and bell dots) and a tooltip on the trigger and
  on its close button. Radix's trigger takes any children, so the slots are
  an API shape rather than a behaviour; the tooltip is gpui's (see Tooltip).
  Reason: a tab shows its program's state and its whole title.
- **Widths** (an addition): with `trigger_width`, a trigger is as wide as its
  label between a minimum and a maximum, shrinks towards the minimum when
  the list runs out of room, and cuts its label with an ellipsis. shadcn's
  triggers are `flex-1`, so in a list stretched wider than its content they
  share its width evenly; ours keep their own width (without
  `trigger_width`, their content's), and the rest of the list stays empty.
  Reason: a tab strip's tabs are as wide as their titles up to a limit,
  never spread across the window.
- **Drag to reorder** (an addition): with `on_move`, a trigger dragged onto
  another trigger of the same list asks the owner to move it there, and a
  2px primary bar marks the side it will land on. Reason: tab strips
  reorder tabs.
- **The `Attached` variant** (an addition): triggers on the background in
  the muted foreground, a hovered inactive one on the accent background, and
  the active one on a surface with rounded top corners and a 2px primary line
  along its top, so it joins the content below. Each trigger may carry its
  own `surface` and `text_color`, used only while it is active. Reason: a
  window's tab strip, whose active tab takes its terminal's colours (a
  profile can theme a tab differently from the window).
- **`Line` marks the active trigger with a 2px primary line along its
  top.** shadcn's line variant underlines it, `bottom-[-5px]`, in the
  foreground colour. Reason: the spec ("Tab list look"), so the line matches
  the attached strip's and a zone header's active line.
- **Triggers fill the list's height.** shadcn's are `h-[calc(100%-1px)]`.
  Reason: gpui has no `calc()`; the difference is a pixel.
- **Horizontal and left-to-right only, automatic activation only, and no
  `TabsContent`.** Radix has `orientation`, `dir` and
  `activationMode="manual"`, and its content panels. Missing features; the
  owner shows what the active trigger stands for.

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
- From recollection, not from a source: browsers firing `contextmenu` on the
  secondary press on Linux and macOS and on its release on Windows,
  floating-ui's `size` middleware leaving out collision padding by default,
  and browsers' tab strips closing a tab on the middle release (`auxclick`).
- The tabs' dark look (shadcn's active pill on `input` at 30% with an
  `input` border) has unit tests of its colours but hasn't been looked at in
  a capture; only the light tokens have.

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
closes (emits `DismissEvent`) before acting on the item. A host whose
closing has side effects in its owner reports it in an event of its own
instead and never emits `DismissEvent`, and the owner closes the slot:
Brindle's palette sends `PaletteEvent::Dismissed`, so the workspace's rules
for ended sessions and orphaned tabs always run.

## Gallery

```sh
cargo run -p brindle-ui --example gallery              # every component, light and dark
cargo run -p brindle-ui --example gallery -- --check   # the spec's scenarios, from inside the app
```

The gallery shows every component in every variant and size, with a dark
toggle, dialogs (one that refuses to be dismissed), popovers in each window
corner, selects and command lists inline, in a dialog and in a popover,
a context menu target and two dropdown menus (one with descriptions and
shortcuts, one of forty items that scrolls), toggles in each variant and
size, buttons and a plain element with tooltips (beside the tooltip's
look drawn in place), and tab lists in each variant: a pill list with a
disabled trigger, a line list, an attached strip above a panel in the active
tab's colours (closable, reorderable triggers with an icon, status and bell
dots, tooltips and a long title cut at the maximum width) and a crowded
strip whose triggers shrink to their minimum.

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
cargo test -p brindle-ui                           # the pure parts: layers, focus cycle, placement, lists, menus, tab keys, styles, tokens, EditState
cargo clippy -p brindle-ui --all-targets           # the gallery included
```

Every decision is a pure function with unit tests; the GPUI glue around
them is thin. GUI behaviour is checked by the gallery's `--check` on a
desktop, and by Brindle's end-to-end suite as each surface adopts a
component.
