# Design

## Context

See `proposal.md` for the decisions. Today (`src/zone_view.rs`): `render_tree` puts a 1px `div` between split children, `render_leaf` returns the bare `TerminalView` for one zone and otherwise a column of a 28px `div` header (global tokens, a 2px `primary` line on the active one) over the terminal. `can_split_zone` counts `ZONE_HEADER_HEIGHT` and `ZONE_DIVIDER_SIZE`. `TmuxWindowView::render` places panes absolutely at `padding + cell * rect`, and the grid it reports to tmux comes from `fit_grid` on the bounds of its own element (`tmux_view.rs`), as does `TerminalElement` from its own bounds. So anything that wraps these views in a card shrinks the grid with no change to `fit_grid`; only code that *estimates* a zone's size by hand (`zone_outer_size`) needs the card's chrome added.

gpui 0.2.2 clips to rectangles (`ContentMask` is bounds only), so `overflow_hidden` plus rounded corners does not round the content. The terminal paints its background to its box, and the default `padding` (6px) is less than the `radius_lg` (10px), so a terminal's square background would show past the card's rounded border.

## Goals / Non-Goals

**Goals:** one `Card` component in `brindle-ui` that both the zone tree and the tmux view use; the same look for an unsplit tab, a split tab and a tmux window; no change to config keys or tmux layout handling.

**Non-Goals:** draggable dividers, the overflow menu, a Button Group or Badge component (tabled in the proposal), changing the header buttons' actions, menu or tooltips.

## Decisions

### 1. `brindle_ui::Card`, with `CardColors` from its owner

`Card::new(id)` is a `RenderOnce` builder like `TabTrigger`: `.header(CardHeader)`, `.child(content)`, `.colors(CardColors { surface, text, muted, border, surround })` (defaults: the card/foreground/muted-foreground/border tokens, `surround` the background token), `.ring(bool)`. `CardHeader` has `.leading(el)`, `.title(label)` (laid out with `trigger_label`, since `truncate()` cuts nothing in gpui 0.2.2) and `.action(el)`. Geometry lives in a pure `card_style(&Tokens)` in `style.rs`, with the ring from the existing `ring_geometry`, unit-tested like `tab_style`. The crate knows nothing of terminals: the app passes the zone terminal's colours.

*Alternative:* build the card in `zone_view.rs` only. Rejected: the tmux view needs the same frame, and the proposal wants the find bar and dialogs able to share it later.

### 2. Rounded corners: surround cutouts above the content

The card paints, after its content, four corner pieces (the square minus a quarter circle, as a `PathBuilder` fill) in `surround`, then draws the border on top. The content is untouched and keeps its square background, so no clipping is needed and the terminal's paint path is unchanged.

*Alternatives:* inset the content by the radius (loses up to 10px of terminal per side, and the padding already is the inset); shrink the radius to the padding (6px; not shadcn's `rounded-lg`, and still wrong when the user sets `padding = 0`). The cutouts work for any padding. A spike (task 1.1) confirms the paint order before the rest is built; if the cutouts cannot be painted above a child, fall back to the radius-capped-to-padding option and amend the spec.

### 3. Surround and border colours

The gap, the outer margin and the corner surround are the window chrome's `background` token, the strip's colour (changed after first sight: the strip, the margin and the gaps read as one surface, and the strip has no border under it). The zone tree and the tmux view paint it as their root background and pass it as `CardColors::surround`. The border is a mix of the zone terminal's background and foreground, using the same mix `Theme::ui_tokens` uses for `border` (`chrome_border`, made `pub(crate)` as `Theme::card_border`) so a card's line reads like a chrome border in that theme. The header sits on the terminal background; its title uses the foreground (active) or muted foreground (`Theme::muted_foreground`, already the tab face's rule), the buttons the muted foreground.

### 4. Layout: gaps replace dividers

`ZoneTreeView`'s root gets 6px padding (`ZONE_GAP`); a split node is a flex container with `gap(ZONE_GAP)`; the divider `div` and `ZONE_DIVIDER_SIZE` go. Normalised `UnitRect` geometry (focus movement, `--dump-screen-after`'s `--- zone … rect=`) is unchanged: it describes the tree, not pixels.

### 5. Header and the active cue

The 2px `primary` line on the active header is dropped: the ring and the title colour are the cues (proposal 7, 12). The header menu, its anchor (`laid_out.headers`, the card's first child), tooltips and `inert` button wrappers stay as they are. A header press still focuses the zone.

### 6. Ring

The ring shows when the tab has 2+ zones, the zone is active, `zone.focus_handle.contains_focused(...)` and `shows_ring(true, modality(cx), false)`. It is drawn in the 6px gap (3px ring), so it never covers a neighbour's border. A one-zone tab shows no ring: it is always "active", so a ring would carry no information and would show on every keyboard focus.

### 7. Sizes

`CARD_BORDER = 1`, `ZONE_GAP = 6`. A zone's outer size (`zone_outer_size`) becomes `grid + 2*padding + 2*CARD_BORDER (+ header)`. `can_split_zone` becomes: a child needs `min_term + 2*CARD_BORDER` wide, `min_term + 2*CARD_BORDER + header` tall, with `ZONE_GAP` between children (it replaces the divider term). The zone spec scenario "Card chrome counts" pins this. The fallback estimate before first paint also subtracts the margins. tmux needs nothing beyond its card: `TmuxWindowView` is wrapped by `Card` with no header, and its existing bounds-based `fit_grid` yields the smaller grid, which is what `refresh-client -C` sends.

### 8. Where tmux's card lives

The card wraps `TmuxWindowView` in `Workspace::render_content` (or inside the view's own `render`; the view's, so the existing e2e geometry reads stay relative to one element). The pane-border colours and title lines are inside the card, unchanged.

## Risks / Trade-offs

- **Corner cutouts misdrawn** (anti-aliasing seam, or drawn under the child) → spike first (task 1.1); the fallback is in Decision 2.
- **Ghost button hover on terminal colours**: `Button` hover surfaces come from the `accent` token, which may clash with a terminal background → check by eye on every built-in theme; if it is illegible, add a hover-colour override to `Button` (listed in `ui/README.md` as a difference).
- **Lost space**: about 14px per dimension per tab, and 6px more per split. Accepted in the proposal; the e2e `zones` case geometry expectations shift.
- **Hit testing at corners**: the cutouts are paint only, so a click in a cutout still reaches the terminal. Harmless (a 10px corner inside the padding).
- **Gap colour with differently themed zones**: the gap is the chrome background whatever a zone's theme, so zones with their own themes need no change.
