# Design

## Context

- See proposal.md for motivation and scope. The surfaces today:
  - `render_tab_bar` (`workspace.rs`) builds the strip from `div`s. Colours come from `active_theme()`, which is the **active tab's** profile theme, through `chrome_background`, `chrome_border`, `muted_foreground` and `hover_background`. The active tab's face is the terminal background, so it joins the terminal below.
  - `ZoneTreeView::render_leaf` draws a header per zone when there are two or more, from the tab's theme; `render_tree` draws the dividers.
  - `find_bar.rs` draws its panel, option toggles and buttons from the terminal's theme.
  - `tmux_view.rs` paints title lines and dividers on a canvas, and uses `chrome_border` for the divider fallback.
  - The window border (`workspace.rs`, client decorations) and `DraggedTab` use the same helpers.
- `ui_tokens()` is itself built from `chrome_background`, `chrome_border` and `muted_foreground`, and the contrast test calls them, so they cannot be deleted; they stop being used by call sites.
- `brindle-ui` has `Button` (variants, sizes `Default/Sm/Lg/Icon`), `Input`, `Popover`, `Select`, `Command`, the `Overlay` slot with layers and focus scope, `Anchored` placement against a trigger rectangle, and five icons. It has no tooltip, toggle, kbd, separator or menu.
- GPUI's `.tooltip(...)` on a stateful element shows a view after a 500 ms hover and hides it on leave and press.
- `⌄` currently calls `open_profile_picker`; the profile list and `open_profile(i)` (with the session-shown-elsewhere flow) live in `picker.rs` and `workspace.rs`.
- Constraints from CLAUDE.md: strip controls must not take focus on a press on Windows (caption hit-testing); GPUI re-renders the window on any `notify`; no synthetic input on the user's desktop, and a pointer press can't be posted on Linux.

## Goals / Non-Goals

**Goals:**
- Every call site outside `theme.rs` reads chrome colours from `cx.ui_theme()`.
- The strip, headers and find bar are composed of `brindle-ui` components wherever a component exists, with no behavioural change except what the specs list (tooltips, menus, the find bar's focus styling).
- Menus and tooltips follow shadcn/Radix, with every difference listed in `ui/README.md`.

**Non-Goals:**
- `TabsContent`, submenus, checkable or radio menu items.
- Changing what a press does: it activates a tab and leaves focus in its terminal.
- Retheming chrome per tab, or UI zoom.

## Decisions

### Chrome follows the global tokens, except each tab's face

The strip and headers read `cx.ui_theme()`, which `ui-theme-tokens` already derives from the global theme, so a profile theme no longer tints the strip as a whole (the spec "Chrome ignores per-tab themes" already said so for components; the strip was an exception). The one place a tab's theme shows is that tab's face: every trigger, active or not, is drawn on its terminal's background in its terminal's foreground (60% opacity while inactive and not hovered), and the active one has the 2px top line in that tab's `accent()`, not `primary`, since `primary` is not guaranteed to read on every theme's background. The strip uses shadcn's `line` variant unchanged in its geometry (`h-9` list, `p-[3px]`, `gap-1`, `px-2`, medium weight, `rounded-md` triggers). The component stays reusable and knows nothing of terminals: `TabTrigger::surface(..)`, `::text_color(..)` and a new `::line_color(..)` are per-trigger overrides that the host fills from each tab's theme. When every tab shares the global theme, which is the usual case, the strip is plain shadcn `line` in that theme's colours. *Alternatives:* (1) the crate's own `attached` variant, with only the active tab in the terminal's colours. Rejected after the first implementation: it looked nothing like shadcn's tabs. (2) `line` with token colours throughout. Rejected: a tab then does not show its profile's theme at all. The strip grows from 30px to about 36px (`h-9`); the caption hit-test and the e2e colour samples follow it.

Mapping: `chrome_background` → `background`; `chrome_border` → `border`; `muted_foreground` → `muted_foreground`; `hover_background` and the pressed/hover mixes → `accent`; `accent()` marks (drag-over bar, active header line, bell dot) → `primary`, while the active tab's top line keeps the tab's own `accent()`; status dots keep the ANSI colours of the **global** theme (`Settings::theme(None)`), with `muted_foreground` for working.

### `theme.rs`: helpers become private

`chrome_background`, `chrome_border`, `muted_foreground` and `hover_background` lose `pub` and anything no longer used is deleted; `ui_tokens()` and its contrast test keep what they need. `Theme::accent()` stays (a config field; `config_edit` and the tmux pane styling use it). The tmux divider fallback `chrome_border().mix(foreground, 0.15)` becomes `tokens.border` mixed with `tokens.foreground`, read in `render`, which has `cx`. Verified by `grep` after the change: no use of those four names outside `theme.rs`.

### A `Tabs` component (`TabList` and `TabTrigger`)

`brindle-ui` gets shadcn's `Tabs` pieces: `TabList` (the `role=tablist` container, one keyboard stop) and `TabTrigger`. `TabsContent` is not built: the content below the strip is the host's workspace. Radix behaviour implemented: roving focus (focus rests on the active trigger), left/right move focus **and activate** (automatic activation, shadcn's default), wrapping, home/end, disabled triggers skipped, activation on a primary press. Variants `Default` (pill) and `Line`, as shadcn has them and no variant of our own; the strip uses `Line`. The list is a `FocusStops`-like roving group: it owns one `FocusHandle`, and the active trigger index decides which trigger shows the ring.

Additions Brindle needs, which Radix lacks, added to the component and listed in `ui/README.md` as differences (the `ui-components` rule): a close button slot inside a trigger (its press doesn't activate), adornment slots (status and bell dots), a min and max width with an ellipsised label, middle-click close, and drag-to-reorder with a drop bar (a trigger is draggable and a drop target; the owner is asked to move it). A fifth difference: **Radix focuses a trigger on a press; ours doesn't.** The host's activation handler focuses the activated tab's terminal, and a press never leaves focus in the strip. This keeps typing going to the terminal after a click and keeps the Windows caption rule (a press that moves focus onto a strip element counts as handled). The strip becomes keyboard-reachable through a hidden `focus_tab_strip` action that focuses the list; Enter and Escape in the list (host-handled, outside the component) return focus to the active terminal. *Alternative:* leave the strip a non-focus-stop and ship only a styled trigger. Rejected by the request: it would not be shadcn's tab control.

### Buttons in the strip

Each is `Button::icon_only(...).variant(Ghost).size(IconXs)` with no `.focus(..)`, so a press neither focuses nor starts a window move: on Linux the strip's drag handler is on the container, so the button sits in a wrapper whose `on_mouse_down(Left)` stops propagation, as the hand-built buttons do today. The close button is the trigger's close slot; `+` is a plain button; `⌄` is the profile dropdown's trigger (below). On Windows the strip `occlude()`s the workspace, unchanged. The native caption buttons (minimize, maximize, close) remain hand-built elements with no handlers on Windows (`window_control_area` only), coloured from `button_style(Ghost, IconXs, tokens)`; a `Button` with an `on_click` installs press handlers, which is what the caption rule forbids there.

### Icons and sizes

`ButtonSize` gains `IconSm` (2rem) and `IconXs` (1.5rem), shadcn's `icon-sm` and `icon-xs`; the icon scales to 0.75 / 0.875 of the button as `Icon` does today. `Icon` gains `Plus`, `Minus`, `ArrowUp`, `ArrowDown`, `Columns2` and `Rows2` from lucide's paths, drawn like the others from polylines; if a path needs a rectangle (the split icons) a `Stroke::Rect` is added to the icon renderer. No chrome control uses a text glyph: `+`, `⌄`, `×`, `⧉` and the caption glyphs (`–`, `□`, `❐`) fall back to whatever font Windows picks and look uneven there, so they are drawn icons (`Plus`, `ChevronDown`, `X`, `AppWindow`, `Minus`, `Square`, `Copy` for restore) at lucide's stroke weight, identical on both platforms. The caption buttons stay hand-built elements with no handlers on Windows, but draw an `IconElement` inside. *Alternative:* the `Segoe Fluent Icons` font on Windows 11 for the native glyphs. Rejected: Windows-only, absent on Windows 10, and a second path to maintain. The find bar's `Aa` and `.*` stay text labels inside `Toggle`s (shadcn's Toggle takes text).

### Tooltip

`Tooltip::new(label).shortcut(keys)` builds a view; an extension `.tooltip_for(label, keys)` on stateful elements calls GPUI's `tooltip`, so the hover delay (500 ms) and hiding on leave and press are GPUI's. It draws as shadcn's: `foreground` background, `background` text, `text-xs`, small padding, arrow omitted. The shortcut is a `Kbd` in the tooltip's inverted colours. No delay group yet (shadcn's `delayDuration={0}` and skip-delay are not replicated; listed in the README as a difference). The text is built on demand when the tooltip is shown, and shortcuts are resolved then through the same lookup the palette uses (`shortcut(action, focus, window)` in `workspace.rs`), so a rebound key shows correctly and `render` pays nothing.

### Kbd, Separator, Toggle

- `Kbd::new("ctrl-shift-t")` splits on `-`, draws keycaps (`muted` background, `muted_foreground`, `text-xs`, `rounded-sm`) with the names unchanged; strokes separated by a space get a gap. It draws the strings the keymap prints; mapping `cmd`/`super` per platform is the keymap's job.
- `Separator` is a `div` of `border` colour, horizontal or vertical.
- `Toggle` mirrors `Button`: same `interaction` and activation helpers, `pressed: bool`, variants default/outline and sizes default/sm/lg from shadcn; pressed is `accent` background with `accent_foreground`. Without `.focus(..)` it takes no focus, as the find bar's buttons today. The find bar passes `Sm`.

### Menus: one `Menu` view, two openers

A `Menu` is the shared content; `ContextMenu` opens it at the pointer on a secondary press, `DropdownMenu` below a trigger on a primary press (Radix opens on press, toggles on a second press, and also on enter, space and down when the trigger is focused). Each is an `Overlay`-hosted `ManagedView` (`Menu`) holding `Vec<MenuEntry>` (`Item { label, shortcut, disabled }` and `Separator`). It emits `MenuEvent::Selected(index)`; every other way out is `DismissEvent`. The host places it with `Anchored` against a zero-size rectangle at the pointer (`Side::Right`, `Align::Start`, offset 0: `place` flips only along the side's axis, so `Bottom` would clamp a menu near the right edge over the pointer instead of opening it to the pointer's left): `placement::place` works for a zero-size trigger, and its flip and clamp keep the menu inside the window. Behaviour is Radix's ContextMenu with these decisions:
- It blocks the pointer outside it (`block_outside_pointer`, as the select's menu does): an outside press only closes it.
- Highlight starts on nothing; down highlights the first enabled item, navigation does not wrap (Radix's `loop` defaults to false), home/end and typeahead as `Select`'s list, shared through `primitives::nav`.
- Pointer: the same gesture rule as `Select` (spec "Select pointer behaviour"): the release ending the opening press is ignored after a move of at most 10px and selects the item under the pointer after a larger one, so press, drag, release is one gesture. A click on an item selects. This is checked against Radix's menu source when implemented, and any difference goes in the README.
- A menu taller than the window gets a maximum height and scrolls (the profile list can be long); the highlight is scrolled into view.
- No submenus, checkable items, labels or arrows.

### The profile menu

`⌄` is a `DropdownMenu` trigger (a ghost `IconXs` button with `ChevronDown`, no focus handle). Its items are the profiles in configuration order, labelled with the profile's name, the picker's description beneath it and `ctrl-alt-N` (from the keymap, for the first nine). Choosing item *i* runs the same path as the picker's choice (`open_profile(i)`, which already handles "session shown here" and "shown by another Brindle"), so none of that logic moves. Rows come from the picker's profile list so the order matches. ctrl-shift-space, `open_profile_picker` and `PaletteKind::Profiles` are untouched.

Each item carries the picker's description (what the profile runs; for a control-mode profile its session and "open in this window") as a second line: `MenuEntry::Item` gets an optional `description`, a documented addition to Radix's menu item that other apps can reuse. The one thing the menu loses against the palette is filtering as you type: a menu has typeahead on the name only, and the filtered list stays on ctrl-shift-space. *Alternative:* a `Popover` holding a `Command` (shadcn's combobox pattern) would filter, but it is the palette in a popover, not a menu, and adds a second picker to keep in step.

### Menus in the workspace

`Workspace` gets a third overlay slot, `context_menu`, with the same one-at-a-time rule as the palette and settings slots (opening one closes the others; a modal slot already blocks the pointer, so a right-click cannot reach the strip while one is open). Opening a tab menu first activates that tab, a zone menu first focuses that zone, so the item's action, dispatched with `window.dispatch_action` after refocusing the tab (as the command palette does), acts on it and follows focus. Item labels and shortcuts come from the `ACTIONS` table (label) and the keymap (shortcut); items carry the action, not a callback. The tab menu's items:

| Tab | Items |
| --- | --- |
| local | Rename Tab · Move Tab Left · Move Tab Right · separator · Close Tab |
| tmux | tmux: Rename Window (shown as "Rename Window") · Move Tab Left · Move Tab Right · separator · Close Tab |

Move Tab Left/Right are disabled at the ends. Close Tab on a tmux tab already closes that tmux window (`close_tab_at`), so the same action serves. The zone menu: Split Zone Right · Split Zone Down · separator · Close Zone; each split item is disabled when `can_split_zone` is false for its axis, using the zone's current outer size.

Right-press handling: on the tab face `on_mouse_down(Right)` opens the menu and stops propagation, so the strip's `show_window_menu` handler (not Windows) doesn't also fire. On Windows only the empty `caption` element is marked `WindowControlArea::Drag`, so a right-press on a tab is not a caption event and the system menu doesn't appear.

### Hidden actions for the keyboard and for tests

`OpenTabMenu`, `OpenZoneMenu`, `OpenProfileMenu` and `FocusTabStrip` (config names `open_tab_menu`, `open_zone_menu`, `open_profile_menu`, `focus_tab_strip`; no palette label, no default key) open the active tab's, the active zone's or the profile menu (beside the tab face, the header or `⌄`) and focus the strip. They give keyboard paths (Radix opens a context menu from the menu key; GPUI has none, so a user may bind one) and let `--action` drive the menus and the strip in e2e, where a secondary or primary press can't be posted. `--dump-screen-after` prints an open menu as `--- menu items=Rename Tab|Move Tab Left|…  highlighted=N` (disabled items in brackets) and `--- focus menu`; `--key down` / `--key enter` then drive it through the keymap.

### Find bar

The panel is `popover` background, `border`, `shadow_md`, `rounded_md` (the `PopoverStyle` look), 400px wide as today. Inside: the existing `TextInput` in an `Input` (flex-1, invalid state when the regex is bad), the count in `muted_foreground` (the invalid message in `destructive`), a `Separator`, two `Toggle`s (`Aa`, `.*`), two ghost `IconXs` `Button`s (`ArrowUp`, `ArrowDown`) and a close `Button` (`X`), each with a tooltip. The panel keeps `occlude`, its mouse-down focusing the query, `key_context("FindBar")` and everything in `FindBar`'s state machine. The focused-panel accent border goes: the `Input` shows the ring instead. The match highlight colours in the terminal are untouched.

### Zone headers and dividers

Header: `background` token, `border` bottom, `text_xs`; title in `foreground` when active and `muted_foreground` otherwise; the 2px active line is `primary`; the three buttons are ghost `IconXs` with `Columns2`, `Rows2` and `X` icons and tooltips. Dividers use the `border` token. Header height stays `ZONE_HEADER_HEIGHT` (28px): `IconXs` is 24px.

### tmux

Title lines and dividers keep canvas painting. The zoomed badge becomes the `secondary` token pair. Divider fallback colours come from tokens as above; `theme.accent()` for the active-pane divider stays, since tmux's style values resolve against the pane's theme.

## Risks / Trade-offs

- [The strip's own background and buttons stop following the active tab's theme, a visible change for per-profile themes] → each tab's face keeps its own theme's colours, so a profile theme still shows on its tab; the spec states the rule. The `!` marker is not used because no config key or default changes.
- [Tab faces in different themes sit side by side, so adjacent faces can clash, and a theme's background can be close to the strip's] → a legible foreground on its own background is a property of each theme (the `ui_tokens` contrast test already covers every built-in theme); the gap and the list's token background separate faces. Judged by eye in the 11.4 check.
- [The strip is 6px taller (`h-9` against 30px)] → the terminal area below shrinks by 6px; the caption hit-test and the e2e capture cases are updated to the new height, and the Windows `win` cases run again.
- [`⌄` loses the picker's filtering] → typeahead on the name remains and the full filtered palette is still on ctrl-shift-space and `open_profile_picker`.
- [Tab strip additions (close, drag, middle-click, adornments, no focus on press) make our Tabs differ from Radix's] → each is added to the component and listed in `ui/README.md`, as the `ui-components` rule requires.
- [A tab list with a focus handle might make GPUI treat a press as handled and break the Windows caption] → the press never focuses the list (handled by the host's activation); `win` e2e cases and a manual check cover it.
- [A `Button` in the strip lets a press start a window move on Linux] → wrapper stops propagation; an e2e case drives the strip actions, and the manual checklist covers a real press.
- [Windows caption: new handlers on tab faces for the right press] → only a right-press handler is added to tab faces, which are not caption areas; Windows is checked by hand and by the `win` cases (which probe the caption hit-test).
- [Tooltip text built on demand could read a stale keymap] → built when shown, so it reads the keymap then.
- [Menu on a background tab activates it, so right-click changes the active tab] → matches Chrome and Firefox's tab menus closely enough; recorded in the spec.
- [Per-frame cost] → tab faces, headers and the find bar build a few more elements; no allocation beyond today's per-frame `SharedString` clones; checked with the `latency-shell` case before and after.

## Migration Plan

Land in one PR as separate commits per task group: components first (each with its gallery entry), then each surface. Nothing is persisted, so no migration; rollback is a revert.

## Open Questions

- Whether the tab menu should later grow items that need new actions (Duplicate Tab, Close Others). They are deferred without affecting this change's specs.
