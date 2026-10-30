# Design

## Context

See proposal.md for motivation and specs/ for required behaviour.

`Marks` already keeps each `Command`'s A/B/C/D positions as absolute lines, through scrolling, pruning and reflow, in local tabs and control-mode panes. Everything else assumes a straight viewport: paint walks `renderable_content().display_iter`, and `cell_at`, links, selection, search and prompt jumps convert through `display_offset`. Alacritty comes from crates.io and offers one `display_offset` for one continuous viewport, so it can't express hidden rows or a second scroll position.

GPUI re-renders the whole window on any `cx.notify()`, so `prepaint` has to stay cheap. On Linux and Windows every wheel event arrives as `TouchPhase::Moved`, so there is no gesture boundary. tmux panes have no padding and must stay on their exact cell grid.

## Goals / Non-Goals

**Goals:**
- One mapping from screen rows to grid lines, used by paint and by every pointer path.
- With no blocks showing, today's behaviour, unchanged.
- Bounded per-frame work, independent of scrollback length.

**Non-Goals:**
- Persisting collapse or scroll state across restarts or reattach; it lives in `Marks` and goes when they do.
- New theme keys or settings dialog fields.
- A scrollbar for the terminal as a whole.
- Default key bindings for the block actions; they are palette entries users can bind.
- Showing marks on the alternate screen, which stays as it is today.

## Decisions

### A projection maps screen rows to grid lines

A pure module, `src/terminal/blocks.rs`, builds a `Projection` from the commands, the view position, the viewport size and the cursor. It holds one entry per screen row: the grid line shown there, and what that row is (plain, header, body, or collapsed header). It is computed in `prepaint` from the commands overlapping the viewport, stored on `GridLayout` beside the cell metrics, and read by event handlers, so a click always resolves against what was painted.

Paint iterates the projection and reads rows with `term.grid()[Line(n)]`, replacing `display_iter`. Selection, match and link checks already work on grid `Point`s, so they carry over once paint hands them the row's real line.

Alternatives: a separate `Term` per command (Warp's approach) would break cursor addressing, the shared scrollback and tmux restore; a fork of alacritty conflicts with using the crates.io release; adding a gutter or real rows changes the PTY size.

### The identity case keeps alacritty's scrolling

`Projection::active` is false when `blocks` is off, the alternate screen is on, or there are no commands. Paint and input then use the existing `display_offset` paths unchanged. This guarantees the "as if blocks did not exist" requirements and keeps the alternate-screen bypass in one place.

**This check comes before any mark is resolved to a grid line.** On the alternate screen `term.grid()` is the alt grid, which has no history, while marks describe the main grid, so reading a block row there would hit the wrong row or go out of bounds. The failed-command bar in `terminal_element.rs` currently has no such check and appears to paint over TUIs; moving it behind the bypass fixes that.

Crossing between the two states converts the position: entering carries over the current top line, and leaving sets `display_offset` from the anchor if the line is still in the grid.

### View position is an anchor on an absolute line

While the projection is active, `display_offset` stays 0 and the terminal holds `ViewAnchor::{Bottom, Line(i64)}`. An absolute line needs no adjustment as output arrives, and it stays meaningful when rows above collapse. `Terminal::scroll` translates `Scroll::Delta`, page, top and bottom into walking visual rows from the anchor. `Terminal::input`'s return to the bottom sets `Bottom`, and `Marks` clamps an anchor that has left the scrollback.

Alternative: an offset in visual rows needs a total row count and has to be rewritten on every collapse above it.

### Inner position lives on `Command`

`Command` gains `collapsed: bool`, `body_top: Option<Pos>` (`None` means follow the tail), `started`/`ended: Option<Instant>` set from the `C` and `D` marks, and `header: Option<Arc<str>>`. `body_top` joins `positions_mut`, so it stays correct across reflow. `Marks::snapshot` and `restore` assume four positions per command, and both move to five.

Absolute lines keep a scrolled body still while output arrives. Duration comes from when Brindle received the `C` and `D` marks, so it is unknown until the command ends.

`header` is captured from the grid when `C` arrives. `prune` keeps a command until its last mark leaves the scrollback, so the prompt rows can go first, and the projection falls back to this text then. The `Marks` lock order (parser, term, marks) is unchanged, and `prepaint` already takes marks under the term lock.

### Height cap and tall headers

The cap follows the spec: viewport rows minus the latest prompt's rows, or the full viewport while the latest command runs. The cap therefore changes by a row or two as a command ends; a following block just shows that many fewer rows at its tail. A fixed viewport-sized cap was rejected because the arriving prompt would push a finished tall block's command off the top.

### Wheel routing is a pure function with an idle timeout

`TerminalView` keeps a `WheelLatch { target: Outer | Block(prompt line), last: Instant }`. A `route(latch, now, hit, can_scroll)` function decides the target and is unit-tested with injected instants. The timeout is a 300 ms constant, used because GPUI gives no gesture boundaries on these platforms. Mouse reporting and alternate scroll keep priority, and the existing `scroll_remainder` logic stays in front of routing.

Alternatives: a modifier makes plain wheel useless inside blocks and clashes with shift's reporting bypass; click-to-focus competes with selection.

### Inline programs and reporting coordinates

While the header would cover the cursor's row or `MOUSE_MODE` is set, the projection for a capped running block is built without a header, which is the exact live screen. Mouse reports convert a screen row to its grid line and then to the program's row, and a row outside the live screen reports nothing.

### Chrome is painted within rows

Header tints join `rects` ahead of background runs, so cells with their own background paint over them. The rail reuses the failed-bar geometry in `Frame::marks`, in the padding when there is room and over the first column otherwise. Chevrons and summaries are shaped like other text and go through GPUI's shaped-line cache. Colors derive from existing theme colors with `Color::mix`, so themes gain no keys. The chevron is a hit target checked before selection in `mouse_down`.

### Verification hooks

`--dump-screen-after` gains `--- block prompt=N rows=A..B state=running|done|collapsed follow=yes|no`, and prints the projected rows rather than the raw viewport when blocks are active. `--action` runs `toggle_block` and the other actions. Wheel latching and the projection are covered by unit tests with no GUI, since the suite doesn't send real input.

## Risks / Trade-offs

- [Every viewport consumer must move to the projection] → Convert them in one pass, keep raw viewport access private to the identity path, and add e2e cases for links, selection and search below a collapsed block.
- [Per-frame cost in tall scrollback] → Walk commands only from the anchor to the bottom of the viewport; `Top` and `Bottom` are the only moves that may walk further, and they run in response to input.
- [Inline TUIs at the cap lose top rows] → The header steps aside on cursor or mouse reporting. A program drawing only to covered rows without moving the cursor there stays covered until the block scrolls.
- [Duration and line counts are approximate for redrawing programs] → They report what is in the grid and when marks arrived; this is shown as information only.
- [Latching can feel unresponsive for 300 ms] → It's a single constant, to tune by hand; wheel feel is added to the manual checks.
- [Windows ConPTY redraws can rewrite rows in place] → Marks already track these rows; the Windows e2e cases get a block check.

## Migration Plan

`blocks` defaults to on and needs no migration. `blocks = false` is the rollback and restores the identity path. `assets/default-config.toml` gains the key, commented like its neighbours.

## Open Questions

- The 300 ms latch timeout and the header tint's mix ratio need tuning by feel once running. Neither changes the specs or the structure.
