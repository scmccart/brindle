# Proposal

Status: skeleton. The main decisions are made (below); specs, design and tasks come after the remaining open questions are settled.

## Why

`ui-chrome` moved the strip, the find bar, the menus and the zone headers' buttons and colours onto `brindle-ui`, but the **zone** itself is still hand-built: a 28px header bar made of `div`s (status dot, title, three ghost buttons) and 1px dividers. There is no `brindle-ui` component for "a titled pane with actions", and no shadcn look for the whole zone.

Split buttons were dropped from the tab strip because they belong on the zones. A **single-zone tab has no zone header** (zones spec, "Unsplit tab has no header bar"), so the first split comes from a key (ctrl-shift-e, ctrl-shift-o), the tab menu or the command palette; the header's split buttons only appear once a tab is already split. That is accepted and stays.

## Decisions made

1. **No header on single-zone tabs**, as today. (Rejected: always showing the header, a 28px row on every tab, and a floating action group in the terminal's corner, which hides output.) The zones spec's "Unsplit tab has no header bar" stays.
2. **Dividers are not draggable.** They stay static, so shadcn's Resizable is not used and the tree's geometry handling does not change. tmux panes keep their canvas-painted dividers (they are tmux's layout).
3. **A zone is a card:** a bordered, rounded surface with a header row and the terminal as its content.
4. **The header uses the terminal's theme**, as tab faces do (background, foreground, icon colours from the zone's terminal). This reverses `ui-chrome`'s rule that zone headers stay on the global tokens, and the `ui-theme-tokens` requirement "Window chrome uses the tokens" has to widen its exception from tab faces to zone headers (and the card around them, see below).

5. **A gap between cards**, so each zone's rounded corners show (no edge-to-edge cards).
6. **The card's border is derived from the terminal's theme**, not the `border` token.
7. **The active zone is marked by a ring** (shadcn's focus ring on the card), to be judged by eye and changed if it does not work.
8. **tmux windows get one card around the whole window**: the panes inside keep tmux's layout, title lines and canvas-painted dividers, so a tmux tab is a single card, not one per pane.
9. **Narrow-zone overflow is tabled.** It is an edge case, so the action slot stays three buttons for now and the overflow menu waits for a later change.

## Ideas (which shadcn pieces fit)

No existing `brindle-ui` component fits as is:

- **Card** (`CardHeader` with `CardTitle` and a `CardAction` slot, `CardContent`): the shape of a zone. Header row (status dot, title, action slot) on the terminal's colours, content flush against the terminal (`p-0`). A new `brindle-ui` component (`Card`), which also gives the find bar, dialogs and future panels one shared shape. Like `TabTrigger`, it takes the surface and text colours from its owner and knows nothing of terminals.
- **Button Group** (joined buttons): split right and split down as one group in the action slot, then close apart from it.
- **Dropdown Menu** (exists as `DropdownMenu`): an overflow `…` in the action slot for rename, move and close, instead of three always-visible buttons when the zone is narrow.
- **Badge**: the status dot with a label for blocked, done and error states, if the dot alone proves too small (see the icon legibility work in `ui-chrome`).

## Open questions

1. **Card sizes:** the gap between cards, the corner radius and the border width cost pixels (a card fights a terminal that fills edge to edge). Pick them in the design from what looks right; what does the terminal's own padding do inside the card?
2. **Cards on unsplit tabs:** a local tab with one zone has no header. Does its terminal still sit in a card (border and rounded corners, no header row), so a tab looks the same before and after a split, and the same as a tmux window's card? Or is the card only drawn for two or more zones, which leaves the unsplit terminal edge to edge?
3. **tmux window card:** one card around the whole window (decision 8). Does it have a header row (the window's name and actions), or only the border and rounding?
4. **The ring's colour and width**, and whether the ring shows on a mouse focus or only on keyboard focus (shadcn's focus-visible), since a zone is focused by clicking it.

## What Changes

- a `brindle-ui` `Card` component (surface and text colours from its owner, a header with a title, an action slot, and content);
- `ZoneTreeView` rebuilt on it for local tabs with two or more zones: each zone a card with a gap between cards, the header on the terminal's theme, the border derived from it, a ring on the active zone, and the dividers replaced by the gaps; `TmuxWindowView` wrapped in one card;
- the zones spec's "Multi-zone chrome and header controls" requirement revised for the card and the header theme (the unsplit-tab rule stays);
- `ui-theme-tokens`: the zone header and card take the terminal's colours.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `zones`: header, divider and card requirements.
- `ui-components`: the `Card` component requirements.
- `ui-theme-tokens`: zone headers (and their cards) join the tab faces as the exception to "chrome uses the tokens".

## Impact

`src/zone_view.rs` (header, dividers, the split tree's render), a new component in `ui/src/components/`, `ui/README.md`, the zone e2e cases and the theme checks. No config keys or defaults change. Depends on `ui-chrome` having landed (it supplies the strip, the icon sizes and the owner-colours pattern from `TabTrigger`).
