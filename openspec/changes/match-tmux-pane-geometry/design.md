# Design

## Context

`TmuxSession::sync_panes` walks each window's parsed layout (`Layout::panes()`, which returns a `Rect` per pane) and resizes every pane's remote `Terminal` to its cell. `TmuxWindowView` positions each pane view at its cell, and draws dividers and the active-pane highlight from the same cells. New panes are restored with three commands: `display-message` with `PANE_STATE_FORMAT`, then `capture-pane -a`, then `capture-pane`. `%output` for the pane is dropped while `Pane::restoring` is set.

From tmux 3.6 (see proposal.md):
- `pane-border-status` changes pane geometry without any `%layout-change`.
- Layout strings never include the reserved row.
- Format subscriptions (`refresh-client -B name:%*:format`) report per-pane values as `%subscription-changed name $s @w idx %p : value`. They are checked about once a second.

In tmux's code, the title row sits at `yoff - 1` (top) or `yoff + sy` (bottom).

## Goals / Non-Goals

**Goals:**
- A pane's grid always ends up tmux's real pane size, and stays in step with tmux's screen afterwards.
- No extra round trips or re-snapshots in the steady state, such as when dragging a split with border status on.

**Non-Goals:**
- Drawing anything in the reserved rows. That belongs to `show-tmux-pane-titles`; this change leaves them as background.
- Removing the up-to-about-1 s lag of subscriptions. It would need polling.

## Decisions

1. **Cells and pane rectangles are tracked separately.** `TmuxSession` keeps a `geometry: HashMap<PaneId, Rect>` holding tmux's reported pane rectangles. A `pane_rect(id, cell)` helper returns the reported rectangle when it lies within the cell, and the cell otherwise. That fallback covers panes not reported yet and stale reports after a layout change. `TmuxWindowView` positions and sizes pane views from `pane_rect`, but keeps the cells for dividers and the active highlight. That way divider adjacency doesn't change, and the reserved row reads as part of the divider area.

2. **Insets carry over across layout changes.** A layout change shouldn't wait for a fresh report. For each pane Brindle stores its inset, which is the number of rows the reported rectangle is short of its cell at the top and at the bottom. On `%layout-change` it applies the stored inset to the new cell straight away. Insets only change when a pane moves to or from a window edge, or when the option changes, and the subscription corrects those cases (decision 4).
   - **Alternative considered:** encode tmux's rule (panes touching the top or bottom edge lose a row, zoomed panes too) from a subscribed `#{pane-border-status}`. That predicts edge moves correctly, but it hard-codes tmux 3.6 behaviour, which has changed between versions, and still needs decision 4 as a backstop. Insets are less code and stay correct across versions.
   - **Alternative considered:** run `list-panes` after every `%layout-change`. Its reply can race the program's redraw, and the size would differ from what `sync_panes` just set, causing a re-snapshot on every layout change.

3. **Initial geometry comes with the pane state.** `PANE_STATE_FORMAT` gains `#{pane_left} #{pane_top} #{pane_width} #{pane_height}`, appended after the existing fields so the field indices `restore_bytes` uses don't move. When the state reply arrives, before the captures, Brindle records the geometry and resizes the terminal. The captures then fill a grid of the right size, so a new pane or a fresh attach is correct from its first frame.

4. **A subscription is the source of truth for later changes.** Once attached (on the first window list), Brindle sends `refresh-client -B 'brindle-geometry:%*:#{pane_left} #{pane_top} #{pane_width} #{pane_height}'` once, as `Pending::Ignore`. `protocol.rs` parses `%subscription-changed` into `Notification::SubscriptionChanged { name, window, pane, value }`; the pane is `None` for `-`. On a geometry value:
   - Update `geometry` and the pane's inset.
   - If the size differs from the terminal's current size, resize the terminal and re-snapshot the pane (decision 5).
   - If only the position differs, re-render. Nothing else is needed.

   A failed `-B` on older tmux logs a warning and degrades to today's behaviour.

5. **A re-snapshot is the existing restore, run again.** Set `restoring` to a fresh `Restore`, which starts dropping `%output`, and send the same three commands. When the snapshot arrives, prefix the replay with `ESC c` (RIS). alacritty's `reset_state` clears both grids and the history, so replaying the snapshot doesn't duplicate scrollback. A pane that is still restoring isn't re-snapshotted again: the newest geometry is applied when its state reply lands.
   - **Alternative considered:** resize only, and wait for the program's next redraw. Programs redraw on SIGWINCH, which tmux already delivered before Brindle heard about the change, so there may be no further redraw.

6. **Subscription plumbing is generic.** `SubscriptionChanged` is dispatched on `name`, so `show-tmux-pane-titles` and `apply-tmux-pane-styles` can add their own subscriptions without touching the parser.

## Risks / Trade-offs

- [A program draws during the up-to-1 s window after the option changes] → It shows at the wrong size briefly. The re-snapshot then replaces it with tmux's screen.
- [Insets are wrong for a moment after panes move between edges, for example `rotate-window` with border status on] → The same subscription and re-snapshot path corrects it within about a second.
- [Re-snapshot cost: `capture-pane -S -` sends the full history] → It only happens when the size changes outside a layout change, which is rare: an option toggle or an edge move with border status on. The restore already does this for every pane on attach.
- [RIS also resets modes and colors set by the program] → `restore_bytes` re-applies the modes tmux reports, the same as on attach. Colors a program set with OSC are lost, as they are on attach today.
