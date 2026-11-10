# Proposal

Status: skeleton. The decisions are made (below, no open questions left); specs, design and tasks are written when the change is picked up, after `ui-chrome` lands.

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

10. **Unsplit local tabs are a card too**, with no header row, so a tab looks the same before and after a split and the same as a tmux window. The zones spec's "no header bar" rule for one zone is unchanged; only the card frame is new.
11. **The tmux window card has no header row** either: a border and rounded corners round the whole window.
12. **The ring shows on keyboard focus only** (shadcn's focus-visible), through the same focus-visible modality `brindle-ui` already tracks. A zone focused with the mouse shows no ring, so the header's title in the foreground (muted when inactive, as today) stays as the cue that works for both.
13. **Sizes (chosen here, adjusted by eye in the design):** a 6px gap between cards and as an outer margin round the card (twice the tab pill's 3px padding), the `rounded-lg` radius of the tokens (`radius_lg`), a 1px border, and the terminal's current padding unchanged inside the card. The card costs about 14px in each dimension against an edge-to-edge terminal (6px margin and 1px border on each side); the design verifies that `fit_grid` and the tmux `refresh-client -C` size account for it.

## Open questions

None. Narrow-zone overflow is tabled (decision 9).

## What Changes

- a `brindle-ui` `Card` component (surface and text colours from its owner, a header with a title, an action slot, and content);
- `ZoneTreeView` rebuilt on it: every local tab's zone a card (headerless when the tab has one zone, with a header on two or more), a gap between cards, the header on the terminal's theme, the border derived from it, a keyboard-focus ring on the active zone, and the dividers replaced by the gaps; `TmuxWindowView` wrapped in one headerless card;
- the zones spec's "Multi-zone chrome and header controls" requirement revised for the card and the header theme, and the unsplit-tab rule amended only to add the card frame;
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
