# Design

## Context

After `match-tmux-pane-geometry`:
- Pane views sit at tmux's pane rectangles.
- The rows a cell has beyond its pane, the reserved title rows, are left to `TmuxWindowView`'s canvas, which paints dividers.
- `Notification::SubscriptionChanged` is routed by name.

tmux 3.6 draws a title line (`screen_redraw_make_pane_status` in `screen-redraw.c`) as follows:
- It fills row `yoff - 1` (top) or `yoff + sy` (bottom) with border characters, using `pane-active-border-style` for the active pane and `pane-border-style` for the others.
- It then draws the expanded `pane-border-format` over that row, starting at `xoff + 2`, with width `sx - 2`.

Tested: a subscription on `#{T:pane-border-format}` gives each pane's expanded string, with `#[...]` directives left in. Conditionals such as `#{?pane_active,#[reverse],}` are already resolved, so the value changes when the active pane changes.

## Goals / Non-Goals

**Goals:**
- Show titles as tmux would, using tmux's own expansion, so custom formats keep working.

**Non-Goals:**
- Mouse interaction with title lines.
- Alignment, range, list and fill directives (`align=`, `range=`, `list=`, `fill=`). They are parsed and dropped.
- The colors of `pane-border-style` itself. That is `apply-tmux-pane-styles`; this change uses theme colors for the base style.

## Decisions

1. **Two more subscriptions.** Alongside the geometry subscription, send:
   - `brindle-title:%*:#{T:pane-border-format}`, giving a per-pane title string;
   - `brindle-border-status:@*:#{pane-border-status}`, giving `off`, `top` or `bottom` per window.

   Store them as `TmuxSession::titles: HashMap<PaneId, Vec<StyledRun>>` and `TmuxWindow::border_status`. Drop entries when their pane or window goes away.
   - Rejected alternative: decide title placement from geometry insets. An inset shows that a row is reserved but not which side the title goes on for panes that aren't on an edge.

2. **A small style-run parser in `src/tmux/style.rs`.** `parse_format(&str) -> Vec<StyledRun>`, where a `StyledRun` has text, `fg`/`bg: Option<TmuxColor>`, `bold` and `reverse`.
   - `#[a,b c]` directives change the current style. Recognised: `fg=`, `bg=`, `bold`/`nobold`, `reverse`/`noreverse`, and `default`/`none`, which reset to the base. Everything else is ignored.
   - `##` is a literal `#`.
   - `TmuxColor` is `Default`, `Indexed(u8)` or `Rgb(Color)`. It accepts tmux names (`black` to `white`, `brightred` and the other bright names), `colourN`/`colorN` and `#rrggbb`. Unknown values count as `Default`.

   `apply-tmux-pane-styles` reuses `TmuxColor` and the style parsing.

3. **Painting happens in the existing canvas, after the dividers.** For each visible pane whose window has border status on:
   - The title row is `rect.y - 1` (top) or `rect.y + rect.height` (bottom), using the pane rectangle, and is skipped if it falls outside the window.
   - Text starts at column `rect.x + 2` and is clipped to `rect.width - 2` columns, measured with `unicode-width`.
   - Each run is shaped separately with the terminal font (`cell_metrics`), placed at its exact column, and painted over a background quad that covers its cells. The divider line then doesn't strike through the text.
   - Indexed colors resolve against the pane's theme palette.
   - **Base style when a run has no `fg`:** the theme foreground for inactive panes and the accent for the active pane. This mirrors tmux's defaults, where border text uses the default color and the active border is highlighted.
   - **Rows of edge panes:** tmux draws border characters across the reserved row of panes on the window edge too. Brindle draws its divider line across that row, so every title sits on a line.

4. **Shaping is cached per title string.** Shaping on every paint would allocate in `prepaint`/`paint`, which `CLAUDE.md` warns against. The view keeps a cache from run text and font size to the shaped line, rebuilt when the subscription value or the font changes.

## Risks / Trade-offs

- [Titles lag up to about 1 s behind `select-pane -T`, because subscriptions are checked on a timer] → Acceptable for a label. tmux's own redraw is also periodic for formats with times.
- [Formats using unsupported directives (`align=right`, `fill=`) look different from tmux] → The text is still shown, left-aligned. This can be extended if real formats need it.
- [Wide characters, such as emoji in agent names, are measured with `unicode-width` and could differ from tmux's widths] → Clipping uses the same measure for every run, so the worst case is one cell of misalignment at the end of the line.
