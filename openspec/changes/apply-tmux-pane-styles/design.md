# Design

## Context

After `match-tmux-pane-geometry` and `show-tmux-pane-titles`:
- `TmuxSession` has subscription routing by name.
- `src/tmux/style.rs` parses tmux style directives and colors (`TmuxColor`).
- Dividers are painted from the theme's divider and accent colors, and title base colors are the theme foreground and accent.

A remote `Terminal` resolves default colors through `default_color(index)` from its `theme`. `ColorRequest` replies, and the frame background, go through the same path. `report-pane-colors-to-tmux` sends each pane's theme foreground and background to tmux with `refresh-client -r`, and tmux prefers that report over `window-style` when it answers OSC 10/11 (`input_osc_10` and `window_pane_get_bg` in tmux 3.6).

Tested on tmux 3.6:
- `window-style` and `window-active-style` are per-pane options. Both border styles are window options.
- `#{option}` in a format gives the raw value, and `#{T:option}` expands it. The built-in `pane-active-border-style` is the format `#{?pane_in_mode,fg=yellow,#{?synchronize-panes,fg=red,fg=green}}`, which expands to `fg=green`.

## Goals / Non-Goals

**Goals:**
- Match tmux's own resolution of default colors for each pane, so Claude Code's per-agent tint and users' tmux styles look the same as in a normal tmux client.

**Non-Goals:**
- Style attributes other than colors (bold, dim and so on through `window-style`), `pane-border-lines` and `pane-border-indicators`.

## Decisions

1. **Subscriptions.**
   - Per pane: `brindle-pane-style:%*:#{T:window-style}|#{T:window-active-style}`.
   - Per window: `brindle-border-style:@*:#{pane-border-style}|#{T:pane-border-style}|#{pane-active-border-style}|#{T:pane-active-border-style}`.

   `|` separates the fields. Style values can't contain it, because tmux styles are comma- or space-separated attribute lists.

2. **The built-in border defaults count as unset.** A border style takes effect only if its raw value differs from tmux's built-in default: `default` for `pane-border-style`, and the format string above for `pane-active-border-style`. The expanded value is used when it does. Otherwise every window would get green active borders instead of the theme accent, and a mode or sync change would flicker the color, which Brindle can't usefully mirror.
   - Rejected alternative: compare the *expanded* value with `fg=green`. That would also ignore a user who deliberately sets green.

3. **Resolving default colors.** A pure `resolve_pane_defaults(window_style, active_style, is_active) -> (Option<TmuxColor>, Option<TmuxColor>)` returns, for each of foreground and background:
   - for the active pane, `window-active-style`'s color if it is set and not `default`;
   - otherwise `window-style`'s color;
   - otherwise `None`, meaning the theme.

   `TmuxSession` stores both style strings per pane. It re-resolves when they change and when the active pane changes. It pushes the result into the pane's `Terminal` as a new `default_override: (Option<Rgb>, Option<Rgb>)`, with indexed colors resolved against the pane's theme palette. `Terminal::default_color` returns an override for `Foreground` and `Background` (and the dim foreground is derived from it), and the theme otherwise. Local PTY terminals never set an override.

4. **Color reports follow the shown colors.** Because tmux checks Brindle's report before `window-style`, the report must carry the effective colors, or a styled pane would answer queries with the theme's. Whenever a pane's override changes, Brindle re-sends that pane's `refresh-client -r` reports with the override colors, falling back to the theme. `color_report_commands` already takes explicit colors, so only the call sites change.

5. **Border colors.** `TmuxWindowView` reads the window's resolved border foregrounds. `paint_dividers` uses them in place of the theme's `normal` and `accent` when they are set. Title base colors from `show-tmux-pane-titles` use the same two colors, so a title sits in its border's color as in tmux.

6. **Names as implemented.**
   - The override is `Terminal::default_colors: DefaultColors`, resolved through a pure `palette_color(theme, defaults, index)` so it can be unit-tested without a GPUI context.
   - The resolver is `style::pane_defaults`, and the border helper is `style::border_color` with tmux's built-in values as constants.
   - `tmux::resolve_color` maps a `TmuxColor` through a theme for the session, the view and titles.
   - `TmuxSession::set_config` now takes the context so it can update pane terminals on reload.
   - Title cache keys hold the base color instead of an active flag, so a border style change re-shapes the titles.

## Risks / Trade-offs

- [A user who copies tmux's default `pane-active-border-style` value verbatim into their config] → It is treated as unset, and Brindle shows its accent. This is harmless.
- [Changing a pane's default background repaints the whole pane, and a `select-pane` while styles differ re-resolves two panes] → It's one entity update per pane per change, outside `render`.
- [Overrides apply after the subscription delay of up to about 1 s] → Programs normally set `window-style` once, when the pane is created.
