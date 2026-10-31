# Design

## Context

See `proposal.md` for motivation and `specs/` for required behaviour.

`Workspace` currently models each tab's content as either `TabContent::Terminal(Entity<TerminalView>)` or `TabContent::Tmux { session, view }`. Local tab operations (`active_terminal`, `active_view`, `automatic_title`, `status_summary`, `set_tab_theme`, `on_terminal_event`, and `snapshot`) assume a 1:1 mapping between a local tab and a single `TerminalView`. Meanwhile, `TmuxWindowView` already demonstrates how a tab-level container entity can own multiple `TerminalView`s, forward `TerminalEvent`s to `Workspace`, and expose a dedicated key context (`TmuxWindow`) for split and focus actions.

Unlike `TmuxWindowView`—which positions `fixed_size = true` panes on a character cell grid dictated by `tmux -C`—zones are native GPUI splits with GPUI header controls around each zone's content, and each terminal resizes its own PTY to fit its zone's inner bounds.

## Goals / Non-Goals

**Goals:**
- A self-contained `ZoneTreeView` entity and pure `ZoneTree` layout model that own zone splitting, closing, directional focus, and header chrome without turning `Workspace` into a pane manager.
- An extensible `ZoneContent` seam (`ZoneContent::Terminal(Entity<TerminalView>)` now) so future non-terminal zone types slot into the same tree and chrome.
- Pure, unit-testable tree manipulation and normalized `[0, 1] x [0, 1]` geometry for directional focus and `--dump-screen-after` verification.
- Backward-compatible `session.toml` persistence within format `VERSION = 1`.

**Non-Goals:**
- Draggable split dividers or non-50/50 split ratios.
- Zone zoom (`ctrl-shift-z` stays bound only in `TmuxWindow`).
- Configurable zone chrome styles (e.g. hover/minimal chrome).
- Zones inside tmux control-mode tabs.

## Decisions

### `ZoneTreeView` owns local tab zones and replaces `TabContent::Terminal`

`TabContent::Terminal(Entity<TerminalView>)` becomes `TabContent::Zones(Entity<ZoneTreeView>)` (including for single-zone tabs and `TabOrigin::Message` tabs). `ZoneTreeView` mirrors `TmuxWindowView`'s boundary with `Workspace`:
- It emits `TerminalEvent` (`Bell`, `Notify`, `Status`, `TitleChanged`) and `Exited` (when the last zone in the tree closes or exits) to `Workspace`.
- It exposes `focus_handle(cx)`, `active_view()`, `active_terminal(cx)`, `automatic_title(snapshot, cx)`, `status_summary(cx)`, `refresh_titles(snapshot, cx)`, and `set_profile(profile, theme, font_size, cx)`.

Rationale: keeping a single `TabContent::Zones` variant for both 1-zone and multi-zone local tabs avoids state transitions in `Workspace` when a tab splits from 1 to 2 zones or collapses from 2 to 1.

Alternative: keeping `TabContent::Terminal` for 1-zone tabs and upgrading to `TabContent::Zones` on first split was rejected because it duplicates event subscriptions and theme/title plumbing in `Workspace`.

### Binary tree with stable `ZoneId` leaves and focus serials

A pure data structure holds the split topology:

```text
ZoneId(u64)   // monotonically assigned within a ZoneTreeView (1, 2, ...)

enum SplitAxis {
    Horizontal, // left / right (SplitZoneRight)
    Vertical,   // top / bottom (SplitZoneDown)
}

enum ZoneTree {
    Leaf(ZoneId),
    Split {
        axis: SplitAxis,
        first: Box<ZoneTree>,
        second: Box<ZoneTree>,
    },
}
```

`ZoneTreeView` stores `tree: ZoneTree`, `zones: HashMap<ZoneId, Zone>`, `active: ZoneId`, and a monotonic `focus_serial: u64`. Each `Zone` holds:
- `id: ZoneId`
- `content: ZoneContent` (`enum ZoneContent { Terminal(Entity<TerminalView>) }`)
- `title: SharedString` (cached automatic title for the header bar)
- `status: Option<State>` (cached OSC 7501 summary for the header dot)
- `last_focused: u64` (updated whenever focus enters the zone)
- `_subscriptions: Vec<Subscription>` (`on_focus_in` and `TerminalEvent` subscriber)

When a leaf is split, `ZoneTree::split(target, axis, new_id)` replaces `Leaf(target)` with `Split { axis, first: Leaf(target), second: Leaf(new_id) }`.
When a leaf is closed, `ZoneTree::remove(target)` replaces the parent `Split` with the sibling subtree and returns the sibling subtree's leaf IDs so `ZoneTreeView` can focus the survivor with the highest `last_focused` serial.

### Normalized `[0, 1] x [0, 1]` rectangles drive directional focus and debug dumps

`ZoneTree::rects()` walks the tree from `UnitRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }`, halving `w` on `Horizontal` splits and `h` on `Vertical` splits, and returns `Vec<(ZoneId, UnitRect)>` in tree order.

`ZoneTree::focus_target(active, direction)` resolves `FocusZoneLeft/Right/Up/Down` purely from these unit rects:
1. Filter candidate leaves whose rect lies strictly in `direction` of the active rect (e.g., for `Right`, `cand.x >= active.x + active.w - EPS`) and whose perpendicular interval overlaps the active rect's perpendicular interval (`min(a_end, b_end) - max(a_start, b_start) > EPS`).
2. Rank candidates by:
   - smallest primary-axis distance from the active rect's leading edge;
   - smallest perpendicular distance between the candidate's center and the active rect's center;
   - tree order (earlier in `rects()` wins ties, so moving right from a full-height left zone into two equal right zones picks the top-right zone).

Rationale: normalized coordinates are exact powers of $1/2$, independent of pixel rounding or font metrics, and trivially unit-tested without a GPUI window.

### Native GPUI flex layout and conditional Option A chrome

`ZoneTreeView::render` sets `.key_context("ZoneTree")` and renders `self.tree`:
- **Single leaf (`zones.len() == 1`):** renders the leaf's `TerminalView` directly (`size_full()`), with no header bar and no divider.
- **Multiple leaves (`zones.len() >= 2`):**
  - Each `Split` renders a `div().flex().size_full().min_w_0().min_h_0()` (`flex_row` for `Horizontal`, `flex_col` for `Vertical`) containing the first child (`flex_1().min_w_0().min_h_0()`), a 1px divider (`w(px(1.0)).h_full()` or `h(px(1.0)).w_full()`, colored with `theme.chrome_border().hsla()`), and the second child (`flex_1().min_w_0().min_h_0()`).
  - Each `Leaf` renders a `div().flex().flex_col().size_full().min_w_0().min_h_0().overflow_hidden()` with:
    1. **Zone header bar** (`h(px(ZONE_HEADER_HEIGHT))`, e.g. `28.0`, `bg(theme.chrome_background().hsla())`, bottom 1px border in `theme.chrome_border().hsla()`, and a top/left accent or foreground text color when `id == self.active`):
       - Status dot (`6px` circle colored by `Zone::status`, matching `Workspace::render_tab_bar`).
       - Title (`flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis()`, showing `Zone::title`, accented/bright when active and muted when inactive). Clicking the header focuses that zone.
       - Action buttons on the right: Split Right (`◫` or `|`), Split Down (`⬒` or `–`), and Close (`×`), using the same compact hover button styling as the tab strip. Clicking a split button on any zone splits *that* zone (or focuses and splits it); clicking `×` closes that zone.
    2. **Zone body** (`div().flex_1().min_h_0().child(terminal_view)`): `TerminalElement` inside `TerminalView` automatically reads its allocated GPUI bounds in `prepaint`, subtracts `config.padding`, and resizes its PTY via `fit_grid`.

Modifier changes (`on_modifiers_changed`) are forwarded to all `TerminalView`s in the tree, just as `TmuxWindowView` does, so `ctrl`-hover link underlines update across unfocused zones.

### Minimum split size guard

Before splitting a zone (whether via action on the active zone or clicking a zone's header button), `ZoneTreeView` checks the target zone's current available pixel bounds (from its `TerminalView`'s last `GridLayout`, or the window's content area when unsplit) against `cell_metrics`:
- A minimum terminal grid needs `2 * cell_width + 2 * padding` width and `2 * line_height + 2 * padding` height (`fit_grid`'s floor).
- With 2+ zones, each zone also needs `ZONE_HEADER_HEIGHT` vertically, and a split adds a `1px` divider.
- If halving the zone along `axis` would leave either child below the minimum width (for `Horizontal`) or minimum height (`ZONE_HEADER_HEIGHT + 2 * line_height + 2 * padding` for `Vertical`), the split is rejected as a no-op.
- Existing zones and session restores are never pruned when a window is resized smaller; `overflow_hidden()` clips them cleanly.

### Session restore schema in `session.toml`

To stay compatible with `VERSION = 1` in `src/session.rs`, `TabState::Local` keeps `profile`, `cwd`, and `title` and adds an optional serialized zone layout:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneNodeState {
    /// For a leaf: its working directory. Omitted for a split.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// True on the leaf that had focus when recorded.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub active: bool,
    /// For a split: `horizontal` or `vertical`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<SplitAxis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<Box<ZoneNodeState>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second: Option<Box<ZoneNodeState>>,
}
```

- When a local tab has **1 zone**, `zones: None` is written (omitted via `skip_serializing_if = "Option::is_none"`), producing the exact same `session.toml` as today.
- When a local tab has **2+ zones**, `cwd` on `TabState::Local` still records the active zone's working directory (so an older Brindle reading the file ignores `zones` and restores the active zone's shell), and `zones: Some(root)` records the recursive tree.

### Debug hooks (`--dump-screen-after`)

In `Workspace::dump_active_screen`:
- If the active tab is a `TabContent::Zones(view)` with 2+ zones, `view.describe_zones()` prints:
  - `--- zones count=<N> active=<ID>`
  - `--- zone id=<ID> rect=<x>,<y>,<w>,<h>[ active]` for each leaf in tree order (formatting floats cleanly, e.g. `0`, `0.5`, `1`, `0.25`).
- Single-zone tabs emit no `--- zones` lines, so every existing e2e test output is unchanged.
- All subsequent lines (screen header, `--- grid`, `--- link`, `--- selection`, `--- status`, screen text, `--- find`) continue to read from `tab.active_terminal(cx)` and `tab.active_view(cx)`.

## Risks / Trade-offs

- **[Focus handoff when splitting, closing, or switching tabs]** → Register `cx.on_focus_in` on every zone's `TerminalView::focus_handle`. When a zone gains focus, update `ZoneTreeView::active`, bump `last_focused`, refresh the tab's automatic title, and call `cx.notify()`. When `Workspace::activate` focuses a tab, `tab.focus_handle(cx)` returns the active zone's `TerminalView::focus_handle`.
- **[TOML table nesting for recursive `ZoneNodeState`]** → Verify `toml::to_string` / `toml::from_str` round-tripping in `src/session.rs` unit tests for 1-zone, 2-zone, and 3-zone nested trees. If `toml` prefers a flat adjacency list over nested `Box<ZoneNodeState>` tables, a `Vec<ZoneEntry>` in pre-order traversal serializes cleanly as an array of inline/standard tables without recursion limits.
- **[Rapidly exiting shells during tab close]** → In `Workspace::remove_tab` and `CloseTab`, drop `ZoneTreeView` (or mark it shutting down) before calling `terminal.shutdown()` on every zone's terminal so `TerminalEvent::Exited` callbacks do not trigger redundant tree re-layouts while the tab is being torn down.

## Migration Plan

No config changes or state file version bumps are required:
- Unsplit tabs serialize identically to existing `VERSION = 1` state files.
- Existing `session.toml` files without `zones` deserialize with `zones = None` and open as a single-zone tab.
- New default keybindings (`ctrl-shift-e`, `ctrl-shift-o`, `ctrl-shift-x`, `alt-left/right/up/down`) are scoped to `ZoneTree`, which is active only in local tabs, so `TmuxWindow` bindings in control-mode tabs are unaffected.
