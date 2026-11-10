# Proposal

Status: skeleton. This holds the problem and the ideas to choose from; specs, design and tasks come after the open questions are settled.

## Why

`ui-chrome` moved the strip, the find bar, the menus and the zone headers' buttons and colours onto `brindle-ui`, but the **zone** itself is still hand-built: a 28px header bar made of `div`s (status dot, title, three ghost buttons) and 1px dividers. There is no `brindle-ui` component for "a titled pane with actions", and no shadcn look for the whole zone.

It also leaves a gap: split buttons were dropped from the tab strip because they belong on the zones, but a **single-zone tab has no zone header** (zones spec, "Unsplit tab has no header bar"). So today the first split has to come from a key (ctrl-shift-e, ctrl-shift-o), the tab menu or the command palette; the header's split buttons only appear once a tab is already split.

## Ideas (which shadcn pieces fit)

No existing `brindle-ui` component fits as is. shadcn has several that map onto parts of a zone:

- **Card** (`CardHeader` with `CardTitle` and a `CardAction` slot, `CardContent`): the closest match to "titled pane with actions at its top right". A zone would be a bordered, rounded surface: header row (status dot, title, action slot), content flush against the terminal (`p-0`). This is a new `brindle-ui` component (`Panel` or `Card`), which also gives the find bar, dialogs and future panels one shared shape.
- **Resizable** (`ResizablePanelGroup`, `ResizablePanel`, `ResizableHandle` with an optional grip): the split container. It would replace the fixed 1px dividers with a handle that can be dragged to resize zones (today dividers are static) and takes a keyboard resize. The zone tree's normalized `[0, 1]` geometry already describes what a panel group needs.
- **Button Group** (joined buttons): split right and split down as one group in the header's action slot, then close apart from it.
- **Dropdown Menu** (exists as `DropdownMenu`): an overflow `…` in the action slot for rename, move, and close, instead of three always-visible buttons when the zone is narrow.
- **Badge**: the status dot with a label for blocked, done and error states, if the dot alone proves too small (see the icon legibility work in `ui-chrome`).

## Open questions

1. **Header on single-zone tabs?** Three ways to give the first split a home:
   - always show the header (consistent, costs a 28px row on every tab, and puts the zone title next to the tab's own title);
   - a compact floating action group in the terminal's top-right corner on hover or focus (no row cost, hides output under it while shown);
   - no header when unsplit, as today (split from keys, tab menu and palette only, which is the current state and what the removal of the strip buttons leaves).
2. **Draggable dividers?** Resizable's handle implies drag-to-resize and keyboard resize. Is that wanted, and does it change how the tree stores sizes and how tmux panes (which are tmux's layout, not ours) are treated? tmux panes stay on the canvas-painted dividers either way.
3. **Card chrome or flat?** A bordered, rounded card per zone separates zones well but costs pixels (border, radius, gap) and fights terminals that fill edge to edge. A flat header with only a divider is closer to today.
4. **Theme:** the zone header stays on the global chrome tokens, as in `ui-chrome`; whether a zone's header should wear its terminal theme like tab faces do is not assumed.

## What Changes

To be decided from the questions above. Likely:
- a `brindle-ui` panel component (Card-shaped) and, if wanted, a resizable split container;
- `ZoneTreeView` rebuilt on them (header, dividers, action slot);
- the zones spec's "Multi-zone chrome and header controls" and "Unsplit tab" requirements revised according to question 1.

## Capabilities

### New Capabilities
- None decided.

### Modified Capabilities
- `zones`: header and divider requirements, and the unsplit-tab rule, depending on the open questions.
- `ui-components`: the panel (and resizable) component requirements, if built.

## Impact

`src/zone_view.rs` (header, dividers, the split tree's render), a new component in `ui/src/components/`, `ui/README.md`, the zone e2e cases. No config keys or defaults change unless dividers become draggable (a new interaction, not a changed default).
