# brindle-ui

Brindle's shared UI components on GPUI 0.2.2, modelled on
[shadcn/ui](https://ui.shadcn.com): design tokens, overlay primitives, a
component set and the single-line `TextInput`. Brindle's own surfaces (the
settings dialog, the palettes, the find bar) are rebuilt on it in later
changes; nothing in the crate is specific to Brindle.

## Boundary

The crate depends on `gpui` and the `log` facade (a failed icon is logged once), and nothing else. It never depends on `brindle`,
`brindle-term-core` or `alacritty_terminal`, and knows nothing of terminals,
config or the app's actions table. The dependency direction is
`brindle → brindle-ui → gpui`, and a new dependency here is a change to that
rule, not a convenience.

## What is in it

- **Tokens** (`theme.rs`): `Tokens`, shadcn's token set (background/foreground
  pairs for card, popover, primary, secondary, muted, accent and destructive,
  plus border, input, ring and the radius), held in the `UiTheme` global and
  read through `cx.ui_theme()` (`ActiveTheme`), by value: the innermost
  theme scope's set while one is drawn, else the window's.
  `cx.window_theme()` reads the window's set whatever the scope (an owner
  inside a scope keeps its focus ring in it), and `cx.scoped_theme()` the
  scope's alone, for what a component carries to a tooltip it builds.
  `Tokens::neutral_light()` and
  `neutral_dark()` are shadcn's neutral sets. Mapping a terminal theme to
  tokens is the app's job. Beside it, the `UiFont` global holds the chrome's
  font family, read through `cx.ui_font()` (`ActiveFont`; GPUI's
  `.SystemUIFont` while unset). The app sets it with the tokens
  (`UiFont::set`) and on its own root; the crate's roots that GPUI draws
  apart from the app's tree (tooltips, the dragged tab) apply it themselves.
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
    close. Its kind is an `OverlayKind`: `Modal` (`Overlay::modal()`, a
    backdrop that blocks the pointer) or `NonModal { blocks_outside }`
    (`Overlay::non_modal()` for a popover). A non-modal slot can block the
    pointer outside its view while open
    (`Overlay::new(OverlayKind::NonModal { blocks_outside: true })`, a
    select's menu): an outside press then only requests dismissal and
    reaches nothing beneath. `Overlay::tokens(Some(set))` (or `set_tokens`
    on a kept slot, and `ContextMenu::set_tokens`) draws the view in that
    set, the scope inside the `deferred` it may be lifted into.
  - `ThemeScope` (`ThemeScope::new(tokens, child)`), a Brindle addition
    with no shadcn counterpart (a web page would scope CSS variables on an
    element): draws `child` with `tokens` as the active set in its layout,
    prepaint and paint, restoring the outer set after each, so the
    innermost of nested scopes wins and list items laid out in prepaint
    follow it too. It swaps the set through the `UiTheme` global's cell, so
    it notifies no one and redraws nothing outside it. It can't reach a
    `deferred` child or a tooltip, both drawn after it has ended: a scope
    goes inside a deferred element (`Overlay::tokens`), and the crate's
    components carry the scope's set to their tooltips (`Tooltip::tokens`,
    filled at render by `Button`, `Toggle`, tab triggers, `Badge` and
    `Response`'s links). `TooltipFor` on a plain element can't, and keeps
    the window's set.
  - `FocusScope` and `FocusStops`: Tab and shift-Tab cycle through the stops a
    view lists, and never leave it. `on_step(callback)` tells the owner
    which stop Tab or shift-Tab just focused (not a focus a press moved),
    so it can bring a stop in a scrolled list into view.
  - `Anchored` and `place`: a box against its trigger on a side, aligned to
    its start, centre or end, flipping and clamping to stay in the window,
    or in a region of it given to `Anchored::within` (`place_within`).
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
    - `FIELD_HEIGHT` (36px, shadcn's `h-9`) is an `Input`'s height. An owner
      gives each row's control line at least that (`min_h`) with the control
      centred (whatever it is: a field, a select, a switch, a checkbox), and
      puts an error message under it at its natural height.
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
      renders in place, as the gallery shows its look. `Tooltip::tokens`
      draws it in a set of its own (see `ThemeScope`), at the window's text
      size.
    - `TabList` (`TabList::new(id).active(ix).children(triggers)`) and
      `TabTrigger` (`TabTrigger::new(id, label)`): shadcn's tab list and
      triggers, with what a window's tab strip adds. The owner keeps the
      triggers and which one is active and is asked to change them:
      `on_activate(&TabActivate { index, by })` (`by` is the pointer or the
      keyboard, so the owner decides where focus goes), `on_close(&index)`
      and `on_move(&TabMove { from, to })` (`from` is the dragged trigger's
      own `ElementId`, since triggers may move during a drag, and `to` the
      index of the trigger it was dropped on); `on_trigger_bounds` reports where
      the triggers were laid out. One look, shadcn's default (pill) list
      (`tab_list_style`); shadcn's `line` variant is deliberately not
      offered. A trigger may carry its own `surface` and `text_color`, drawn
      whether it is active or not (`tab_style`). The
      list is one keyboard stop through the handle `.focus(&handle)` gives
      it, with focus resting on the active trigger: left and right activate
      the previous and next enabled trigger, wrapping, home/PageUp and
      end/PageDown the first and last (`tab_key`, `tab_target`). A primary
      press asks to activate and moves no focus. A trigger takes leading
      elements (an icon, status and bell dots), trailing adornments, a tooltip,
      `disabled` (half opacity, skipped, asks nothing) and `closable`: a
      ghost `IconXs` close button (its X 16px at a 2px stroke, in the
      trigger's text colour) shown on the active trigger and on a hovered
      one, whose press asks to close and doesn't activate; a middle
      press on a closable trigger asks to close it too. With
      `on_context_menu`, a secondary press on an enabled trigger asks for
      its menu (`TabContextMenu`: the index and the press, for
      `ContextMenu::open_on_press`) and goes no further. `trigger_width`
      (`TabWidth::new(min, max)`) lets triggers shrink to the minimum and
      grow with their label to the maximum, cutting it with an ellipsis
      (`trigger_label`, public for other one-line labels in a flex row,
      since gpui 0.2.2's `truncate` cuts nothing in a flex item).
      With `on_move`, a trigger dragged onto another of the same list
      (`DraggedTab`, carrying the trigger's id and previewed as a pill with
      its label) asks to move there, and a 2px primary bar marks the side it
      will land on (`drop_target`, from where the dragged trigger is now).
      `edge_bar(EdgeBar::new(fraction, color))` lays a 2px bar
      (`EDGE_BAR_HEIGHT`) over the bottom edge of a trigger, inside its
      rounded corners, filling `fraction` of the width from the left (a
      progress bar); it takes no room and no press.
      `badge(Badge::new(label, color))` puts a badge right after the
      label, before the adornments; as the trigger narrows, the label is cut
      first, down to its first `TITLE_KEEPS` (8) characters (`kept_title`,
      measured in the trigger's font), then a label badge (an icon badge
      isn't cut).

    Four of the builders are not controls:
    - `Badge` (`Badge::new("web", color)`): shadcn's badge, outline
      variant, its text and border in the one colour given (`text-xs`,
      `font-medium`, `rounded-md`, `px-1.5`). It is placed by
      `TabTrigger::badge` and `CardHeader::badge`, which cut it with an
      ellipsis once the title beside it is down to what it keeps.
      `Badge::icon(Icon::TriangleAlert, color)` is an icon alone (16px, a
      2px stroke) in that colour, with no box, and isn't cut. Either takes a
      `tooltip(text)`.
    - `Separator` (`Separator::horizontal()` / `vertical()`): a one-pixel
      rule in the border colour that takes no focus and no pointer events.
    - `Kbd` (`Kbd::new("ctrl-shift-t")`): keycaps from a keymap string, one
      per key with the names unchanged. Strokes separated by a space
      (`ctrl-k ctrl-c`) are groups with a wider gap between them. `keycaps`
      does the splitting, `.colors(background, foreground)` replaces the
      muted tokens (an inverted tooltip). It prints what the keymap prints;
      mapping `cmd` or `super` per platform is the keymap's job.
    - `Card` (`Card::new(id).header(CardHeader::new(id).title(..)).child(content)`):
      shadcn's card, compact, for a titled pane: a bordered, `rounded-lg`
      surface (`card_style`; a 1px `CARD_BORDER`) with an optional header
      row over its content, which fills the rest of the card with no
      padding. `CardHeader` is a `CARD_HEADER_HEIGHT` (28px) row with a
      `leading` adornment (a status dot), the `title` on one line cut with
      an ellipsis (`trigger_label`), drawn muted with `title_muted(true)`,
      and `action`s at its end (icon buttons) over a border line, with an
      optional `edge_bar` (an `EdgeBar`, as a tab trigger's) along its
      bottom edge, above the line, and an optional `badge` after the title
      (cut as on a tab trigger; the actions then keep to the end through an
      auto margin). The slot's
      text colour is the muted one, which a `Button` takes only when given
      it (`text_color`), since a button sets its own. It is a stateful element, so the owner gives it handlers
      (a press, `on_context_menu`); `Card::on_header_bounds` reports where it
      was laid out. `.colors(CardColors { surface, text, muted, border,
      surround, ring })` takes the owner's colours (a terminal's), each defaulting
      to the card, foreground, muted foreground, border and background
      tokens, and the ring to the tokens' focus ring (a card in a theme
      scope is given the window's). `.ring(true)` draws the focus ring outside the border; the
      owner decides when (`shows_ring`). The card paints the four pieces
      outside its rounded edge in `surround` above its content and below
      its border (`corner_cutouts`), so a content that paints a square
      background to its corners (a terminal) stays inside the rounded
      border with no clipping. Refine its layout (`size_full`, `flex_1`)
      through `Styled`.
  - entities the owner creates, keeps and subscribes to: `Select`
    (`SelectEvent::Changed`, only on a change), `Combobox`
    (`ComboboxEvent::Changed`, only on a change) and `Command`
    (`CommandEvent::Confirmed` / `Cancelled`), plus `TextInput`
    (`TextInputEvent::Changed`).
    - **Keys:** every item of a list the owner can replace has an `ItemKey`,
      its identity as shadcn's `value` is: `CommandItem`, `SelectOption`
      and `MenuItem` take their label as the key unless given one
      (`.key(..)`), which an owner whose labels can repeat does. A
      component keeps an item by its key across a refresh and names the
      chosen item by it: `SelectEvent::Changed(key)`,
      `ComboboxEvent::Changed(key)` and `MenuEvent::Selected(key)`. A
      select's or combobox's choice is kept by key (`set_options` keeps it
      only if an option still has it, else the placeholder shows), and the
      owner finds a key among its items as they are then (`option_keyed`,
      `item_keyed`, `entry_keyed`). An index is valid only in the call that
      computed it: `CommandEvent::Confirmed(index)` is read in the same
      update, and an index an owner passes in (`Select::new`,
      `set_selected`) is turned into the key at once.

    `Combobox` is shadcn's combobox, a popover
    holding a `Command`: a trigger drawn as a select's that opens, below it
    and at least its width, a searchable list of options in
    `CommandGroup`s with the chosen one highlighted and checked
    (`CommandItem::checked`); enter or a click chooses, escape or an outside
    press closes, and a character typed on the closed trigger opens it
    searching for that character. `Combobox::set_groups(groups, selected,
    cx)` replaces the options, and an open list keeps its search and
    highlighted option, by its key. The panel is placed only once the
    trigger has been measured (`panel_width(None, min)` is the least
    width). `Command::set_groups(groups, keep, cx)`
    replaces the items and leaves the filter text alone: with `keep: None`
    the first enabled match is highlighted and the list returns to the top,
    and with `keep: Some(key)`, an `ItemKey` the new groups have, that item
    stays highlighted while it matches and is enabled (else the first enabled
    match is) and the list keeps its scroll offset, moving only as far as the
    highlighted row needs to be in view (`CommandRows::scroll_after`, a
    `Scroll`). A key the new groups don't have is as `None`.
    `CommandRows::build` takes the kept item's index in the new groups.
    `Popover` is a `RenderOnce` whose open state
    lives in keyed state; its trigger closure wires `PopoverTrigger::on_click`
    into the trigger's own click.
    - Menus: `Menu` is the content both openers show, a list of `MenuEntry`
      (`Item(MenuItem)` with a label, an optional `description` line under
      it in the muted colour, an optional `shortcut` drawn as `Kbd` keycaps
      and `disabled`; or `Separator`, which has no label and no key). It
      emits `MenuEvent::Selected(key)`, the chosen item's `ItemKey`, which
      the owner finds among its current entries (`entry_keyed`); Escape and
      an outside press only close it. It
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
  `Square`, `Copy`, `AppWindow`, `ArrowUp`, `ArrowDown`, `Columns2`, `Rows2`,
  `Lock`, `TriangleAlert`, `ClipboardPaste`, `Ban`, and for the AI elements
  `Send`, `Brain`, `FileText`, `Pencil`, `Trash`, `Move`, `Terminal`,
  `Globe`, `Wrench`, `CircleAlert` and `LoaderCircle` (lucide's arcs and
  curves flattened into chords once, the path data quoted beside each).
  `IconElement::turn` (and `Button::icon_turn`) draws one turned about its
  centre, a spinner's frame under gpui's `with_animation`. Strokes have butt caps and miter joins, since
  gpui doesn't expose lucide's round caps (so `TriangleAlert`'s dot is a
  short rule). Lucide's stroke (2 on its 24 grid) scales with the
  icon, which leaves a 16px icon 1.33px thin; `IconElement::min_stroke`
  (and `Button::icon_min_stroke`, with `Button::icon_size` for a bigger
  icon than the button size's) sets a minimum in pixels, and
  `LEGIBLE_ICON_STROKE` (2px) is the one window chrome uses: a tab strip's
  buttons, the caption buttons, a trigger's close button and an icon badge. Other icons
  keep lucide's weight.
- **`TextInput`** (`text_input.rs`) and **`filter`** (`fuzzy_match`, the
  case-insensitive subsequence match palettes use).
- **`TextArea`** (`text_area.rs`), the multi-line text input: the same
  `EditState` in `Lines::Many` (`\r\n` and `\r` become `\n`, tab stays,
  other controls go), wrapped at its width with gpui's `shape_text`, growing
  to `max_lines` rows (`DEFAULT_MAX_LINES`, 8) and then scrolling to keep
  the cursor in view. Up and down move by shown row keeping a goal column
  (`motion_vertical` over the pure `WrapMap` of the last layout), home and
  end go to the shown row's ends, shift-enter breaks the line. It binds no
  escape, tab or shift-tab, passes enter on (`TextAreaEnter`) unless an
  input method is composing (`composing()`), and passes up at its first row
  and down at its last, so an owner's keys reach them. The IME window sits
  at the cursor on its wrapped row. `set_read_only(true)` keeps the text,
  takes no edits and draws no caret. `TextAreaEvent::Changed` follows every
  edit the person makes (`set_text` sends none). **`Textarea`**
  (`components/textarea.rs`) is `Input`'s frame around one: the input
  border, the ring while focused, the destructive colour when invalid, at
  least 4rem tall, and a press in its padding focuses the area with the
  cursor at the end.
- **The AI elements** (`components/`), named after shadcn/ui's AI Elements
  and drawn from the tokens; they know nothing of agents:
  - `MessageScroller` (an entity): gpui's `list` with
    `ListAlignment::Bottom`, laying out only the items in view and 512px
    around. The owner keeps the items, renders one by index
    (`MessageScroller::new(count, render_item, cx)`) and reports changes
    with `splice(range, count)` (a changed item is replaced by itself) or
    `reset(count)`. `Follow` (`AtEnd`, `Away`) comes from the list's scroll
    events (`Follow::after_scroll`): at the end it stays there as items are
    added or grow; away, the items in view stay put and an outline
    `arrow-down` button shows, which scrolls to the end and follows again
    (`scroll_to_end`). `scroll_page(PageDirection)` is the owner's page keys'
    way in (`page_scroll`, pure), and a page down that reaches the end
    follows it (`Follow::after_draw`). A top row (an agent zone's "Earlier
    messages") is the owner's to render (`with_top_row(render_top)`) and to
    show (`show_top_row(bool)`, `refresh_top_row()` after it changes): it is
    the list's first item, which the owner's indices don't count, so a page
    up from the first item's top scrolls it into view.
    `top_row_in_view()` says whether the last frame showed it, and
    `on_top_row_view(callback)` is told when that changes. (GPUI's list
    draws its top padding above whichever item is at its scroll position,
    so the row can't live in the padding.) Where the view is as items
    change, its rules (OpenSpec change `agents-zone-focus-and-scroll`):
    (1) following, the end stays in view whatever changes; away, (2) a
    splice before the item at the top shifts its index and keeps its
    offset, (3) one after it leaves it, (4) one that replaces it keeps its
    index (or the last new item) and its offset, so an item growing at the
    top keeps its lines in view, and (5) one that removes it puts the next
    item at the top; (6) an insert at the owner's start leaves whether the
    row is in view to the next frame, and away with the row the first item
    in view puts the item that was first at the view's top, moving up by
    what showed of the row; (7) an item at the top that shrank to its
    offset or less gives the top to the next one; (8) `remeasure_all()`
    measures every item again (a text size or font the owner changed),
    keeping the item at the top at the same fraction of its height. Rules 7
    and 8 settle after the frame that measured the item, one frame late.
    The pure parts are `top_after_splice`, `settle_anchor` and
    `inserts_at_start`. `reveal(ix)` scrolls the least distance that shows
    an item whole, or its top when it is taller than the view, from what
    the last frame showed (the pure `reveal_plan`), following the end again
    when that shows the end; an item drawn whole stays put. An item the
    last frame didn't draw takes a second frame when it is below the view,
    since gpui knows its height only once it is drawn.
    `on_items_view(callback)` tells the owner its items with any part in
    view, after each frame that changed them (not while a reveal is still
    moving the view).
  - `Message` and `Bubble`: one turn's text, the person's (`MessageFrom::User`)
    at the end on `muted`, as wide as its text up to four fifths of the
    row, the other side's (`Assistant`) at the start on the card with a
    border, filling the row. A long message drawn as several list items
    uses `BubblePart::{First, Middle, Last}`, which round, border and pad
    only the outer edges (`BubblePart::edges`); the other side's parts all
    fill the row, so their edges line up into one bubble.
    `Message::action(..)` adds an element (a copy button) under a whole
    message or its last part, drawn only while the pointer is over the
    message.
  - `PromptInput` (an entity owning its `TextArea`): a queue row, the
    `Textarea`, and a toolbar with a reason line (`set_reason`, wrapping at
    the room the toolbar gives it, then an optional small outline action
    button at the end of that room, `set_action(Some(label))`) and
    the submit button, which reads the owner's `PromptStatus`: `Ready` an
    `arrow-up` (disabled while the text is blank), `Submitted` a turning
    `loader-circle` (disabled), `Streaming` a `square` that asks to stop,
    `Disabled` the arrow disabled and the text read-only. It emits
    `PromptInputEvent::{Submit(text), Stop, RemoveQueued, ResendQueued, Action}`
    and never changes its own text: the owner clears it
    (`set_text("")`) once a prompt is taken. The pure rules are
    `button_request`, `enter_submits`, `escape_stops` and `is_blank`. The
    queue row (`set_queued(Some(Queued::..))`) shows the text's first line
    cut with an ellipsis, then `queued` and a ghost `x` (remove) for
    `Queued::Waiting`, or `not sent: REASON` (in `set_held_color`'s colour,
    destructive by default) with ghost `send` (send again, disabled with its
    `Resend::Unavailable` reason as the tooltip) and `x` (discard) for
    `Queued::Held`. Its focus stops for the owner's scope: the text area
    (`Focusable`), the reason line's action while it shows
    (`action_stop()`), the row's enabled buttons (`row_stops()`), and the stop
    button while streaming (`stop_button()`); `focused(window, cx)` says
    which of them has focus (`PromptFocus`), for an owner that reports it.
  - `Reasoning`: a trigger (lucide `brain`, the owner's label, a chevron
    turned over while open) over muted content; the owner holds `open` and
    flips it from `on_toggle`.
  - `Tool`: one row, an icon, the title cut with an ellipsis, an optional
    `.summary` (muted) and a status badge (`ToolStatus`: pending, running,
    done, failed in the destructive colour, `tool_status_colors`), both
    kept whole. With `.open(Some(open))` it ends in a chevron turned over
    while open, and a primary press asks the owner to flip it
    (`.on_toggle`, the row needing an `.id`); the owner draws the content.
    `.open(None)`: no chevron, and a press does nothing.
  - `Diff` (`components/diff.rs`): one file's change from plain lines the
    owner gives (`DiffRowData`: `DiffSign`, old and new line numbers, text,
    `hunk_start`): a header (`.header(op, path, from)`: the operation's
    word, the path, `from → path` for a move or copy, and a pointer-only
    ghost copy button, tooltip "Copy diff", `COPY_DIFF`, asking the owner
    through `.on_copy`), then each line's two numbers, sign column (` `,
    `+`, `−`) and text in the owner's family (`.font`), on `muted`,
    `diff_added` or `diff_removed` (`diff_line_background`), a `border`
    line above a hunk's first line. `.joined(above, below)` drops the frame's
    edge where the owner continues it in another block. No focus, no
    diffing.
  - `Confirmation`: a title and, while `ConfirmationState::Requested`, one
    small button per `ConfirmationAction` (keyed by `ItemKey`, a focus stop
    through the owner's handle), `on_action(key)`; once `Accepted`,
    `Rejected` or `Cancelled`, the outcome (`check` or `x` and the label,
    `ConfirmationState::outcome`) and no buttons. An action may have a
    `description` (plain text under its button, wrapping at the
    confirmation's width; any description stacks the actions,
    `actions_stack`, each button at its own width) and be `disabled`
    (muted, no stop, asking nothing). `.tone(ConfirmationTone::Destructive)`
    marks a request as a risk: while `Requested` it draws a warning icon
    (lucide `triangle-alert`) before the title, the `destructive_tint`
    background and a `destructive` border; once answered it draws as the
    default tone (`confirmation_frame` decides).
  - `Loader` (`components/loader.rs`): a small spinner, lucide
    `loader-circle` turned a twelfth of a turn per `step` (of
    `LOADER_STEPS`), at `rems(0.5)` by default. It asks for no animation
    frame: the owner moves `step` from a clock of its own and re-renders.
  - `Shimmer` (`components/shimmer.rs`): one line of muted text with a
    brighter band at `position` (in characters, `shimmer::levels`), cut
    with an ellipsis. Like `Loader`, the owner moves the position; no run
    is dimmer than `muted_foreground`.
  - `Alert`: shadcn's alert (`AlertVariant::{Default, Destructive}`,
    `alert_style`), lucide `circle-alert` by default, with the children as
    the description.
  - `Response` (`components/response.rs`): rich text the owner gives as
    plain data, `Response::new(id, blocks)` over `Arc<RichBlock>`s. The data
    types (`RichBlock`, `HeadingLevel`, `ListKind`, `ListItem`, `Task`,
    `Code`, `Table` with `ColumnAlign`, `Inline` with `RunStyle` runs and
    `InlineLink`s, checked by `Inline::new`) hold no entity or callback, and
    the crate reads no markup: the app's Markdown renderer builds them.
    Paragraphs and headings are one `StyledText` each (`text_runs`, over
    `spans`): strong bold, emphasis italic, strikethrough struck, code in
    the owner's family (`.code_font`) in the foreground on `muted`, muted
    runs in `muted_foreground`, links in the `link` token, underlined;
    headings bold at 1.25, 1.125 and 1 rem (levels 1–3) and the size
    around them after. Lists have a marker column (`list_marker`: `•`, the
    number from the list's `start`, a task mark drawn as a checkbox but
    taking no focus) with nested blocks indented beside it; quotes a
    `border` bar and `muted_foreground` text; rules a `border` line; code
    blocks are `CodeBlock`s; tables a grid of equal-width columns
    (`grid_cols(n)`) in a `border` frame, the header bold with a line
    under it, lines between rows, each column aligned from its
    `ColumnAlign`, cells wrapping. Links: a press and release on the same
    link calls `.on_link(|link, modifiers, …|)` with the modifiers held at
    the release; the pointing hand shows over a link only while the
    owner's `.link_modifiers(Modifiers)` are held (`holds_modifiers`), and
    a link with a `tooltip` shows it on hover (`tooltip_at`). A pointer
    move redraws when the held modifiers change that; for a still pointer,
    the owner redraws on its own `on_modifiers_changed`.
  - `CodeBlock` (`components/code_block.rs`): a rounded `border` frame
    whose header, on the surface around it, holds the label in
    `muted_foreground` on one line (`trigger_label`: cut with an ellipsis
    when it doesn't fit, never wrapped) and a ghost `IconXs` lucide `copy`
    button that keeps its size (tooltip
    "Copy code", `COPY_CODE`), over the code in the owner's family
    (`.font`) in the foreground on `muted`, keeping its breaks and spaces
    and wrapping long lines (`shown_code` drops the one line break that
    ends it). The button is always shown, takes no focus and keeps focus
    where it was on a press; it gives the owner the whole text
    (`.on_copy`, or a response's `.on_copy_code`) and never writes the
    clipboard. `.joined(above, below)` draws one run of a longer text the
    owner lays out as several blocks: joined above, no header; a joined
    edge has no border, rounding or padding.
- **Actions** (`actions.rs`): `brindle_ui::TextInput*`, `TextArea*`,
  `PromptSubmit`/`PromptStop`, `Dismiss`, `FocusNext`/`FocusPrev` and
  `ListUp`/`ListDown`/`ListFirst`/`ListLast`/`ListConfirm`, bound by
  `default_bindings()` in the key contexts `TextInput`, `TextArea`,
  `PromptInput`, `UiLayer`, `UiCommand`, `UiFocusScope` and `UiList`. A text
  area shares every editing key of a text input but home and end.

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

Combobox (shadcn's is its Popover with a Command inside, a recipe rather
than a component):

- **The trigger is drawn as a select's**, with a down chevron. shadcn's
  example uses an outline button with `ChevronsUpDown`. Reason: in a form
  beside selects (the settings dialog), the two look alike; the crate has no
  `ChevronsUpDown` icon.
- **Choosing the chosen option again keeps it.** shadcn's example clears the
  value on that. Reason: a field that must hold a value (a font family) has
  no empty state.
- **It opens like a select**: on the press, and on enter, space, up and down
  key-downs; a typed character opens it searching. A Radix popover trigger
  opens on its click only. Reason: as for `Select` (no click handler, so
  gpui's keyboard click on key-up never reopens it).

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
- **A secondary press asks for the trigger's menu** (an addition): with
  `on_context_menu`, a secondary press on an enabled trigger tells the owner
  which trigger and hands it the press, moves no focus and goes no further
  (an ancestor's secondary press, such as a strip's window menu, doesn't
  run); without the handler it bubbles on as before. It neither activates
  the trigger nor opens a menu itself: the owner does both, typically
  activating the trigger and opening a `ContextMenu` on the press. Radix's
  Tabs have no menu; one would wrap the trigger in a `ContextMenu.Trigger`.
  Reason: a tab strip's tabs have a menu (rename, move, close), and the list
  draws the triggers, so it is the only place that can hear the press.
- **Adornments and a tooltip** (additions): leading elements (an icon,
  status and bell dots), trailing adornments and a tooltip on the trigger and
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
- **Per-trigger colours** (an addition): a trigger may carry its own
  `surface` and `text_color`. It is drawn on its surface in its text colour
  (its icons and close button too) whether active or not: the active one's
  text at full opacity and the trigger still raised with `shadow-sm`, an
  inactive one's at 60% until hovered, then full. Colours not given come
  from the tokens, so a list without them looks as shadcn's does. Reason: a
  window's tab strip draws each tab in its own terminal's background and
  foreground (a profile can theme a tab differently from the window),
  background tabs included.
- **An edge bar** (an addition): `TabTrigger::edge_bar` lays a 2px bar
  (`EdgeBar`) over the bottom edge of the trigger, inside its rounded
  corners, filling a share of its width from the left in the owner's
  colour; the trigger keeps its size and the bar takes no press. Reason: a
  tab shows its program's progress (OSC 9;4), as Windows Terminal's tabs
  do.
- **A badge** (an addition): `TabTrigger::badge` puts a `Badge` after the
  label. The label gives way first, down to its first 8 characters, and
  then a label badge, which (unlike shadcn's `shrink-0` badge) is cut with
  an ellipsis; an icon badge (`Badge::icon`, an addition: no box, and a
  tooltip) keeps its size. The trigger keeps its height. Reason: a tab
  names the context its program runs in (OSC 3008: a container, or a
  warning for `run0`), and a long label shouldn't hide that, nor a long
  badge the label.
- **A window's strip drops the pill's surface (owner styling, not the component).** Brindle's strip gives its `TabList` a transparent background, no left padding and a gap between faces, through `Styled`, since each face has its own surface. The component still draws shadcn's pill unless told otherwise.
- **One look, the pill.** shadcn also has a `line` variant (no surfaces, an
  underline under the active trigger); it isn't offered. Reason: the spec
  ("Tab list look"): hand checking found it far from shadcn's base look,
  and nothing here needs it.
- **Triggers fill the list's height.** shadcn's are `h-[calc(100%-1px)]`.
  Reason: gpui has no `calc()`; the difference is a pixel.
- **Horizontal and left-to-right only, automatic activation only, and no
  `TabsContent`.** Radix has `orientation`, `dir` and
  `activationMode="manual"`, and its content panels. Missing features; the
  owner shows what the active trigger stands for.

Card (`Card` and `CardHeader`; shadcn's new-york-v4 `card`, which is plain
markup with no Radix primitive under it). Compared against shadcn's `card`
source from recollection while implementing the zones-shadcn change. What
matches: a `bg-card text-card-foreground` surface with a 1px `border`, a
header with a title and an action slot at its end (`CardAction`), and the
content below it. The differences, mostly what a terminal pane needs:

- **Rounded corners are painted over the content**, not clipped. shadcn's
  card relies on CSS rounding and `overflow` (or on its padding keeping the
  content off the corners). Reason: gpui 0.2.2 clips to rectangles only,
  and a terminal paints a square background to its box with less padding
  than the radius, so the card paints the area outside its rounded edge in
  the owner's `surround` colour above the content and below the border.
  The pieces are paint only: a press on one reaches the content beneath.
- **`rounded-lg`, not `rounded-xl`, and no `shadow-sm`.** Reason: cards sit
  edge to edge in a window, 6px apart, where a larger radius eats terminal
  cells and a shadow would fall on the neighbour.
- **Compact and flush.** shadcn's card is `py-6 gap-6` with `px-6`
  sections; here the content has no padding at all and the header is one
  28px `text-xs` row (`pl-2.5 pr-0.5 gap-1.5`). Reason: the content is a
  terminal, whose own padding is the inset.
- **The header always has a border line under it.** shadcn draws one only
  with `.border-b` on the header. Reason: the header and a terminal's
  first row would otherwise run together.
- **No `CardDescription`, `CardFooter` or separate `CardTitle`/
  `CardContent`.** The title is a `CardHeader` property, laid out with
  `trigger_label` (one line, ellipsis), and the content is the card's
  children. Missing features nothing needs yet.
- **A leading adornment** (an addition): before the title, a status dot.
- **An edge bar** (an addition): `CardHeader::edge_bar`, the tab trigger's
  `EdgeBar` along the header's bottom edge, above its line, taking no room.
  Reason: a pane shows its program's progress, as its tab does.
- **A badge** (an addition): `CardHeader::badge`, after the title and cut
  as on a tab trigger. Reason: a pane names its program's context, as its
  tab does.
- **Owner colours** (an addition): `CardColors` replaces the surface, text,
  muted, border and surround colours one by one; the ring stays the
  tokens'. Reason: a pane is drawn in its terminal's colours, as a tab
  face is.
- **The ring is a prop** (`ring(bool)`), not `focus-visible` on the card.
  Reason: the card isn't focusable itself; its owner knows whether the
  pane inside has keyboard focus.

**Textarea and text area.** shadcn's `Textarea` is a `<textarea>`; here
the editing is the crate's own, so these are differences from a browser's:

- **Enter is the owner's.** A text area binds enter to `TextAreaEnter`,
  which passes it on unless an input method is composing; a line break is
  shift-enter. Reason: every owner (a prompt) sends on enter, and a
  browser's textarea has no way to say "composing" to its page's keys.
- **A lone enter or tab no owner binds inserts nothing.** gpui hands an
  unbound enter or tab to the input handler as `\n` or `\t`; the area drops
  those (`is_unbound_key_text`), so a key reaching no binding does nothing
  instead of typing. Pasted and composed text keeps its breaks and tabs.
- **End on a wrapped row stops before the wrap**, so the cursor stays on
  that row (a browser shows it at the next row's start).
- **Up at the first row and down at the last go to the owner** (a browser
  moves to the text's start or end); with shift they extend the selection
  to the start or end, as a browser does.
- **No ctrl-shift-e.** `Input`'s extra select-to-end key isn't a text
  area's (shift-end is). Reason: Brindle's prompt input is a text area in
  an agent zone, where ctrl-shift-e is the zone's Split Terminal Right, and
  the text area's binding, nearer the focus, would take it.
- **Read-only, not disabled:** `set_read_only` keeps the text, takes no
  edits and draws it at half opacity with no caret.
  Reason: a prompt input shows its draft while it can't send.

**AI elements.** Anatomy and behaviour after shadcn/ui's AI Elements; no
code comes from there.

- **The scroll button is outline**, not ghost: it floats over the
  messages, which a ghost button's transparent face would show through.
  It takes no focus; the owner's keyboard path is `scroll_to_end`.
- **A change that contains the list's top item resets the offset to that
  item's start** (gpui's `ListState::splice`). An owner splices only the
  items that changed, so a growing last item moves nothing above it.
- **The other side's message keeps a bubble and fills the row.** AI
  Elements gives the assistant's content the full width with no bubble
  around it, and caps only the person's bubble (`max-w-[80%]`), as here.
  Brindle keeps a bordered bubble on the card behind the other side's
  content, so a conversation's turns read apart on a zone's card, and
  makes it fill the row: a message drawn in parts (one list item per
  block) then has straight edges whatever each part's text.
- **Hover actions are drawn only while the pointer is over the message**,
  so they take no focus; the owner gives each a keyboard path (an action).
- **The prompt input's toolbar is under the frame**, not inside it, with
  the reason line at its start. Reason: the frame is `Textarea`'s, and the
  key context that makes enter submit wraps the text area alone, so the
  row's and the toolbar's buttons keep their own enter.
- **Enter submits while streaming too.** AI Elements' button is a stop
  then; enter in the text asks the owner to submit, which queues it.
- **The submit button is a focus stop only while streaming** (as a stop
  button); ready, enter in the text area does what it would.
- **The spinner turns `loader-circle`** with gpui's `with_animation`.
- **The held label's colour is the owner's** (`set_held_color`): the
  tokens have no warning colour; it defaults to destructive.
- **The queue row** has no AI Elements counterpart (an addition): a text
  queued apart from the draft, which none of its buttons touch.
- **The reason line's action** has no AI Elements counterpart (an
  addition): one labelled button after the reason, for what to do while
  the input can't send (an agent zone's Continue conversation). It is a
  focus stop right after the text area, so it is one Tab away.
- **Reasoning's trigger is pointer-only** (AI Elements' is a button): a
  conversation holds many, and Tab through each would bury the prompt. The
  owner holds `open`; no auto-close timer.
- **Tool is one row** with no collapsible of its own, and takes no
  focus; its badge says pending, running, done or failed. The owner holds
  whether its content is open and draws it below the row (an agent zone
  lays a long detail out as separate list items), so the row is only the
  trigger, pointer-only as Reasoning's.
- **Diff** has no AI Elements counterpart; its line tints are tokens
  (`diff_added`, `diff_removed`), not Tailwind's greens and reds, so a
  theme scope's set draws them.
- **Confirmation's outcome shows the chosen action's label**, and
  `Cancelled` shows "Cancelled"; the actions are keyed (`ItemKey`). An
  action can carry a description under its button (AI Elements' has
  none), which stacks the actions: an agent's login methods are chosen
  by what each does.
- **Confirmation's destructive tone** has no AI Elements counterpart: a
  pending request takes shadcn's destructive Alert look (the icon in the
  destructive colour, a destructive border) but over `destructive_tint`,
  the card mixed at most 12 % toward `destructive` while `card_foreground`
  keeps 4.5:1 and `destructive` 3:1 on it, where shadcn's destructive
  alert keeps the card background. A tint tells the row apart in a long
  conversation; answered, it is plain again.
- **Loader and Shimmer step from the owner's clock** (shadcn's `Spinner`
  and AI Elements' `Shimmer` animate in CSS): gpui 0.2.2's repeating
  animation asks for a frame per display frame, and with no view caching
  each is a whole-window render, so the owner steps them 10 times a
  second and only while they show.
- **Alert's title wraps** (shadcn clamps it to one line): an app's titles
  name a failure and its cause and are read whole.
- **Response reads no markup** (AI Elements' renders Markdown through
  Streamdown): it draws blocks the owner parsed, so the crate keeps its
  closed dependency list and knows no Markdown. It renders **no HTML**
  (the owner hands raw HTML over as text) and **loads no images** (the
  owner hands an image over as a muted text with a link).
- **No syntax highlighting** in code blocks or inline code; no Mermaid
  diagrams and no math.
- **The link gesture is the owner's:** a press on a link is reported with
  its modifiers and the owner decides (Brindle opens on ctrl-click, as in
  its terminals); the pointing hand shows only while the owner's modifiers
  are held, and links are no focus stops.
- **The code block's copy button is always shown** (not on hover) and is
  **pointer-only**: a conversation holds many, and the owner gives the
  keyboard one path (an action that copies the last code block). It
  reports the text and never writes the clipboard.
- **Table columns are equal-width** (a browser sizes them to their
  content), and cells wrap; there is no horizontal scroll.

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

## Notes for component authors

Two quirks of gpui 0.2.2's text measure, which keeps a text's first
measure for any later measure without a wrap width in the same layout:

- **Ellipsis pitfall:** `truncate()`/`text_ellipsis` on a flex item cuts
  nothing (the text keeps its first, natural-width measure). A one-line
  label that must give way in a flex row uses `trigger_label` (a line clamp
  of 1 laid over an invisible unwrapped copy), as tab triggers, tool rows
  and code block labels do.
- **Wrapping text pitfall:** in a flex row, a text item that may wrap
  takes its width from the row (`flex_1().min_w_0()`: grow from a zero
  basis) or has a definite width (`w_full()`, `w(..)`). A one-line label
  uses `trigger_label`, or `flex_none()` when it must never shrink. A bare
  `min_w_0()` on a text item with an automatic basis is the defect: its
  basis comes from its own measure, which can be a narrower one taken
  earlier in the layout, and the text then wraps at that width (a code
  block's label measured 0px wide, one letter a line). The same holds for
  a wrapping text sized to fit in a column (`items_start`): give it the
  column's width.

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
look drawn in place), and pill tab lists: one with a disabled trigger, a
plain one, a strip above a panel in the active tab's colours (closable,
reorderable triggers in their own colours with an icon, status and bell
dots, tooltips and a long title cut at the maximum width) and a crowded
strip whose triggers shrink to their minimum, and cards: the default look
with a header, two in a terminal's colours on a surround of their own (one
with its title muted), a headerless one whose content paints a square
background to its corners, and a pair without and with the ring, and the
AI elements: a conversation (messages on both sides, one in three parts,
tool rows, copy actions on hover) in a message scroller over a prompt input
whose submits join it, and beside them a reasoning block, tool rows in
three states, a loader and a shimmer moved 10 times a second (still under
`--check`), a destructive permission confirmation, an answered one and the
two alerts, and a response
in an agent's bubble with every block kind (headings, every inline style,
links with and without tooltips, nested numbered and bulleted lists, task
marks, a quote, a labelled and an unlabelled code block, a rule, a table
with each alignment and a short row, an image's text) beside a code block
on its own.

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
them is thin. The text area and the AI elements also have `TestAppContext`
tests (gpui's `test-support`, a dev-dependency only), which drive keys,
clicks and input-method text through a headless window
(`test_support::app()`). gpui keeps a `debug_selector`'s last bounds from
frame to frame, so a test that something is no longer drawn records the
paint itself. GUI behaviour is checked by the gallery's `--check` on a
desktop, and by Brindle's end-to-end suite as each surface adopts a
component.
